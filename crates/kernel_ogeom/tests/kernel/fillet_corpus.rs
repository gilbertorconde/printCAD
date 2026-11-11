//! Fillets and chamfers on printable parts, each case named and its
//! volume held to a closed form where one exists, else between bounds:
//! box corners and edges, a pyramid's apex, pocket floors, a boss's foot
//! and top, an obround and a kidney pocket, a cone's foot and top, tangent
//! chains, a round on a round, a 4 degree kink, a slanted bar into
//! flanges. A refusal the kernel owes is an ignored test naming its issue.

use std::f64::consts::PI;

use kernel_api::{
    BooleanOp, ChamferSpec, EdgeProbe, EdgeSelection, ExtrudeTermination, Placement, PrimitiveKind,
    Profile, ProfilePlane, ProfileSegment, ProfileWire, SolidOp, SweepKind,
};

use super::common::{circle_wire, is_sound, plane_at_z, rect_wire, try_chain, volume, xy_plane};

fn prism(plane: ProfilePlane, wires: Vec<ProfileWire>, height: f64, op: BooleanOp) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile { plane, wires },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: height },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op,
    }
}
fn poly(points: &[[f64; 2]]) -> ProfileWire {
    ProfileWire {
        names: Vec::new(),
        segments: (0..points.len())
            .map(|i| ProfileSegment::Line {
                start: points[i],
                end: points[(i + 1) % points.len()],
            })
            .collect(),
    }
}
fn arc(start: [f64; 2], mid: [f64; 2], end: [f64; 2]) -> ProfileSegment {
    ProfileSegment::Arc { start, mid, end }
}
fn line(start: [f64; 2], end: [f64; 2]) -> ProfileSegment {
    ProfileSegment::Line { start, end }
}
fn wire(segments: Vec<ProfileSegment>) -> ProfileWire {
    ProfileWire {
        names: Vec::new(),
        segments,
    }
}
fn at(point: [f64; 3], direction: [f64; 3]) -> EdgeProbe {
    EdgeProbe {
        point,
        direction,
        faces: [0, 0],
    }
}
fn fillet(radius: f64, edges: EdgeSelection, follow: bool) -> SolidOp {
    SolidOp::Fillet {
        radius,
        edges,
        follow_tangent: follow,
    }
}
fn chamfer(distance: f64, edges: EdgeSelection, follow: bool) -> SolidOp {
    SolidOp::Chamfer {
        spec: ChamferSpec::EqualDistance { distance },
        flip: false,
        edges,
        follow_tangent: follow,
    }
}
fn picked(probes: &[([f64; 3], [f64; 3])]) -> EdgeSelection {
    EdgeSelection::Picked(probes.iter().map(|(p, d)| at(*p, *d)).collect())
}
fn rounded_rect(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> ProfileWire {
    let s = std::f64::consts::FRAC_1_SQRT_2 * r;
    wire(vec![
        line([x0 + r, y0], [x1 - r, y0]),
        arc([x1 - r, y0], [x1 - r + s, y0 + r - s], [x1, y0 + r]),
        line([x1, y0 + r], [x1, y1 - r]),
        arc([x1, y1 - r], [x1 - r + s, y1 - r + s], [x1 - r, y1]),
        line([x1 - r, y1], [x0 + r, y1]),
        arc([x0 + r, y1], [x0 + r - s, y1 - r + s], [x0, y1 - r]),
        line([x0, y1 - r], [x0, y0 + r]),
        arc([x0, y0 + r], [x0 + r - s, y0 + r - s], [x0 + r, y0]),
    ])
}
/// The solids the cases dress up.
#[derive(Clone, Copy, Debug)]
enum Base {
    /// 20 x 20 x 10.
    Block,
    /// 20 x 20 x 2.
    Thin,
    /// A 40 x 40 x 5 plate under a boss of radius 6, 8 high.
    Boss,
    /// The plate under a cone from radius 6 to 2, 8 high.
    Cone,
    /// The plate with a boss of radius 6 whose side runs tangent into the
    /// plate's side at x = 20.
    TangentBoss,
    /// 30 x 30 x 10 with a 10 x 10 pocket 5 deep.
    Pocket,
    /// 30 x 30 x 10 with an obround pocket 5 deep: 14 long between its
    /// ends of radius 3.
    Lever,
    /// 30 x 30 x 10 with a kidney pocket 5 deep: radii 6 to 12 over 120
    /// degrees about (15, 8), its ends of radius 3.
    Kidney,
    /// A square pyramid on 20 x 20, its apex 10 up.
    Pyramid,
    /// A 30 x 20 rounded rectangle of corner radius 4, 10 high.
    RoundedRect,
    /// A 40 x 10 block whose front side turns 4 degrees at x = 20, 10 high.
    Kink,
    /// Two 4 mm flanges 40 apart, 30 high and 10 deep, and a 6 mm bar
    /// between them rising 13 across.
    Slanted,
}

/// The kidney's ends turn about its middle radius at these angles.
const KIDNEY: ([f64; 2], f64, f64) = ([15.0, 8.0], PI / 6.0, 5.0 * PI / 6.0);
/// The kink's turn.
const KINK: f64 = 4.0 * PI / 180.0;

fn ops(base: Base) -> Vec<SolidOp> {
    let plate = || {
        prism(
            xy_plane(),
            vec![rect_wire(-20.0, -20.0, 20.0, 20.0)],
            5.0,
            BooleanOp::NewSolid,
        )
    };
    let holed = |pocket: ProfileWire| {
        vec![
            prism(
                xy_plane(),
                vec![rect_wire(0.0, 0.0, 30.0, 30.0)],
                10.0,
                BooleanOp::NewSolid,
            ),
            prism(plane_at_z(5.0), vec![pocket], 5.0, BooleanOp::Cut),
        ]
    };
    match base {
        Base::Block => vec![prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            10.0,
            BooleanOp::NewSolid,
        )],
        Base::Thin => vec![prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            2.0,
            BooleanOp::NewSolid,
        )],
        Base::Boss => vec![
            plate(),
            prism(
                plane_at_z(5.0),
                vec![circle_wire(0.0, 0.0, 6.0)],
                8.0,
                BooleanOp::Fuse,
            ),
        ],
        Base::TangentBoss => vec![
            plate(),
            prism(
                plane_at_z(5.0),
                vec![circle_wire(14.0, 0.0, 6.0)],
                8.0,
                BooleanOp::Fuse,
            ),
        ],
        Base::Cone => vec![
            plate(),
            SolidOp::Primitive {
                kind: PrimitiveKind::Cone {
                    radius1: 6.0,
                    radius2: 2.0,
                    height: 8.0,
                    angle_deg: 360.0,
                },
                placement: Placement {
                    origin: [0.0, 0.0, 5.0],
                    ..Placement::default()
                },
                op: BooleanOp::Fuse,
            },
        ],
        Base::Pocket => holed(rect_wire(10.0, 10.0, 20.0, 20.0)),
        Base::Lever => holed(wire(vec![
            line([8.0, 12.0], [22.0, 12.0]),
            arc([22.0, 12.0], [25.0, 15.0], [22.0, 18.0]),
            line([22.0, 18.0], [8.0, 18.0]),
            arc([8.0, 18.0], [5.0, 15.0], [8.0, 12.0]),
        ])),
        Base::Kidney => {
            let (c, a0, a1) = KIDNEY;
            let p = |r: f64, a: f64| [c[0] + r * a.cos(), c[1] + r * a.sin()];
            let am = (a0 + a1) / 2.0;
            let end = |a: f64, out: f64| {
                let m = p(9.0, a);
                [m[0] - 3.0 * out * a.sin(), m[1] + 3.0 * out * a.cos()]
            };
            holed(wire(vec![
                arc(p(12.0, a0), p(12.0, am), p(12.0, a1)),
                arc(p(12.0, a1), end(a1, 1.0), p(6.0, a1)),
                arc(p(6.0, a1), p(6.0, am), p(6.0, a0)),
                arc(p(6.0, a0), end(a0, -1.0), p(12.0, a0)),
            ]))
        }
        Base::Pyramid => vec![SolidOp::Primitive {
            kind: PrimitiveKind::Wedge {
                xmin: 0.0,
                xmax: 20.0,
                ymin: 0.0,
                ymax: 10.0,
                zmin: 0.0,
                zmax: 20.0,
                x2min: 10.0,
                x2max: 10.0,
                z2min: 10.0,
                z2max: 10.0,
            },
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        }],
        Base::RoundedRect => vec![prism(
            xy_plane(),
            vec![rounded_rect(0.0, 0.0, 30.0, 20.0, 4.0)],
            10.0,
            BooleanOp::NewSolid,
        )],
        Base::Kink => vec![prism(
            xy_plane(),
            vec![poly(&[
                [0.0, 0.0],
                [20.0, 0.0],
                [40.0, 20.0 * KINK.tan()],
                [40.0, 10.0],
                [0.0, 10.0],
            ])],
            10.0,
            BooleanOp::NewSolid,
        )],
        Base::Slanted => {
            let xz = ProfilePlane {
                origin: [0.0; 3],
                x_axis: [1.0, 0.0, 0.0],
                y_axis: [0.0, 0.0, 1.0],
                normal: [0.0, -1.0, 0.0],
            };
            vec![
                prism(
                    xz,
                    vec![
                        rect_wire(0.0, 0.0, 4.0, 30.0),
                        rect_wire(36.0, 0.0, 40.0, 30.0),
                    ],
                    10.0,
                    BooleanOp::NewSolid,
                ),
                prism(
                    xz,
                    vec![poly(&[[2.0, 4.0], [38.0, 17.0], [38.0, 23.0], [2.0, 10.0]])],
                    10.0,
                    BooleanOp::Fuse,
                ),
            ]
        }
    }
}

/// What a round of radius 1 takes off or fills in, per unit length of a
/// straight right-angled edge: the square less the quarter disc.
const Q: f64 = 1.0 - PI / 4.0;
/// How far that section's centroid stands from the edge's two faces, for
/// radius 1.
fn qx() -> f64 {
    (5.0 / 6.0 - PI / 4.0) / Q
}

/// Section of a round of radius 1 on an edge whose faces open at `angle`
/// (radians) on the round's side.
fn round_section(angle: f64) -> f64 {
    1.0 / (angle / 2.0).tan() - (PI - angle) / 2.0
}

/// The bases' volumes.
fn base_volume(base: Base) -> f64 {
    let plate = 40.0 * 40.0 * 5.0;
    match base {
        Base::Block => 4000.0,
        Base::Thin => 800.0,
        Base::Boss | Base::TangentBoss => plate + PI * 36.0 * 8.0,
        Base::Cone => plate + PI * 8.0 / 3.0 * (36.0 + 12.0 + 4.0),
        Base::Pocket => 9000.0 - 500.0,
        Base::Lever => 9000.0 - (14.0 * 6.0 + PI * 9.0) * 5.0,
        Base::Kidney => 9000.0 - (PI / 3.0 * (144.0 - 36.0) + PI * 9.0) * 5.0,
        Base::Pyramid => 20.0 * 20.0 * 10.0 / 3.0,
        Base::RoundedRect => (600.0 - (4.0 - PI) * 16.0) * 10.0,
        Base::Kink => (400.0 - 10.0 * KINK.tan() * 20.0) * 10.0,
        // The bar's 36 x 6 less the 2 x 6 it reaches into each flange.
        Base::Slanted => (2.0 * 4.0 * 30.0 + 36.0 * 6.0 - 2.0 * 2.0 * 6.0) * 10.0,
    }
}

/// What a case should come to.
enum Want {
    /// This volume, within a millionth of it.
    Volume(f64),
    /// A volume strictly between these.
    Between(f64, f64),
    /// Refused: no ball of the radius seats there at all.
    Refused,
}

/// A named case: its base, the dress-up on it, and what it should come to.
struct Case {
    name: &'static str,
    base: Base,
    op: SolidOp,
    want: Want,
}

const X: [f64; 3] = [1.0, 0.0, 0.0];
const Y: [f64; 3] = [0.0, 1.0, 0.0];
const Z: [f64; 3] = [0.0, 0.0, 1.0];

/// The block's corner at the origin's three edges.
fn corner() -> EdgeSelection {
    picked(&[
        ([10.0, 0.0, 10.0], X),
        ([0.0, 10.0, 10.0], Y),
        ([0.0, 0.0, 5.0], Z),
    ])
}

/// The pyramid's four edges running to its apex.
fn apex_edges() -> Vec<([f64; 3], [f64; 3])> {
    [[5.0, 5.0], [15.0, 5.0], [15.0, 15.0], [5.0, 15.0]]
        .iter()
        .map(|q| ([q[0], 5.0, q[1]], [10.0 - q[0], 5.0, 10.0 - q[1]]))
        .collect()
}

/// The kidney floor's four arcs, by the face they bound.
fn kidney_floor() -> EdgeSelection {
    EdgeSelection::OfFaces(vec![[15.0, 17.0, 5.0]])
}

/// The rounded rectangle's top loop, from one side and its tangent chain.
fn top_loop() -> EdgeSelection {
    picked(&[([15.0, 0.0, 10.0], X)])
}

fn cases() -> Vec<Case> {
    use Base::*;
    use Want::*;
    let case = |name, base, op, want| Case {
        name,
        base,
        op,
        want,
    };
    let b = base_volume;
    let rounded_box = |a: f64, w: f64, h: f64, r: f64| {
        let (a, w, h) = (a - 2.0 * r, w - 2.0 * r, h - 2.0 * r);
        a * w * h
            + 2.0 * r * (a * w + a * h + w * h)
            + PI * r * r * (a + w + h)
            + 4.0 / 3.0 * PI * r.powi(3)
    };
    // A loop of straight runs `straight` long and arcs of `turn` radians at
    // radius `about` (measured to the edge), a section `area` whose centroid
    // stands `off` from the edge, toward the arcs' centres when `inward`.
    let swept = |area: f64, off: f64, straight: f64, turn: f64, about: f64, inward: bool| {
        let r = if inward { about - off } else { about + off };
        area * (straight + turn * r)
    };
    let foot_angle = PI - (8.0f64 / 4.0).atan();
    let kink_open = PI - KINK;
    vec![
        case(
            "box edge fillet",
            Block,
            fillet(2.0, picked(&[([10.0, 0.0, 10.0], X)]), false),
            Volume(b(Block) - 20.0 * 4.0 * Q),
        ),
        case(
            "box corner fillet",
            Block,
            fillet(2.0, corner(), false),
            Volume(b(Block) - 44.0 * 4.0 * Q - 8.0 * (1.0 - PI / 6.0)),
        ),
        case(
            "box every edge fillet",
            Block,
            fillet(2.0, EdgeSelection::All, false),
            Volume(rounded_box(20.0, 20.0, 10.0, 2.0)),
        ),
        case(
            "box edge chamfer",
            Block,
            chamfer(2.0, picked(&[([10.0, 0.0, 10.0], X)]), false),
            Volume(b(Block) - 40.0),
        ),
        // Three prisms, less their pairwise overlaps (d³/3 each), plus the
        // three's (d³/4).
        case(
            "box corner chamfer",
            Block,
            chamfer(2.0, corner(), false),
            Volume(b(Block) - (50.0 * 2.0 - 8.0 + 2.0)),
        ),
        case(
            "box every edge chamfer",
            Block,
            chamfer(1.0, EdgeSelection::All, false),
            Volume(b(Block) - (2.0 * 50.0 - 6.0)),
        ),
        case(
            "pyramid apex chamfer",
            Pyramid,
            chamfer(1.0, picked(&apex_edges()), false),
            Between(b(Pyramid) - 2.0 * 300f64.sqrt(), b(Pyramid)),
        ),
        case(
            "pocket floor fillet",
            Pocket,
            fillet(1.0, EdgeSelection::OfFaces(vec![[15.0, 15.0, 5.0]]), false),
            Volume(b(Pocket) + swept(Q, qx(), 40.0 - 8.0 * qx(), 0.0, 0.0, false)),
        ),
        case(
            "pocket floor chamfer",
            Pocket,
            chamfer(1.0, EdgeSelection::OfFaces(vec![[15.0, 15.0, 5.0]]), false),
            Volume(b(Pocket) + 0.5 * (40.0 - 8.0 / 3.0)),
        ),
        case(
            "boss foot fillet",
            Boss,
            fillet(1.5, EdgeSelection::Near(vec![[0.0, -6.0, 5.0]]), false),
            Volume(b(Boss) + swept(2.25 * Q, 1.5 * qx(), 0.0, 2.0 * PI, 6.0, false)),
        ),
        case(
            "boss foot chamfer",
            Boss,
            chamfer(1.5, EdgeSelection::Near(vec![[0.0, -6.0, 5.0]]), false),
            Volume(b(Boss) + swept(1.125, 0.5, 0.0, 2.0 * PI, 6.0, false)),
        ),
        case(
            "boss top fillet",
            Boss,
            fillet(1.5, EdgeSelection::Near(vec![[0.0, -6.0, 13.0]]), false),
            Volume(b(Boss) - swept(2.25 * Q, 1.5 * qx(), 0.0, 2.0 * PI, 6.0, true)),
        ),
        case(
            "lever pocket floor fillet",
            Lever,
            fillet(1.0, EdgeSelection::OfFaces(vec![[15.0, 15.0, 5.0]]), false),
            Volume(b(Lever) + swept(Q, qx(), 28.0, 2.0 * PI, 3.0, true)),
        ),
        case(
            "lever pocket floor chamfer",
            Lever,
            chamfer(1.0, EdgeSelection::OfFaces(vec![[15.0, 15.0, 5.0]]), false),
            Volume(b(Lever) + swept(0.5, 1.0 / 3.0, 28.0, 2.0 * PI, 3.0, true)),
        ),
        case(
            "lever pocket rim fillet",
            Lever,
            fillet(1.0, picked(&[([15.0, 12.0, 10.0], X)]), true),
            Volume(b(Lever) - swept(Q, qx(), 28.0, 2.0 * PI, 3.0, false)),
        ),
        case(
            "kidney pocket floor chamfer",
            Kidney,
            chamfer(1.0, kidney_floor(), false),
            Volume(b(Kidney) + 0.5 * (12.0 * PI + 2.0 * PI * (3.0 - 1.0 / 3.0))),
        ),
        case(
            "cone foot fillet",
            Cone,
            fillet(1.0, EdgeSelection::Near(vec![[0.0, -6.0, 5.0]]), false),
            Between(
                b(Cone) + 2.0 * PI * 6.0 * round_section(foot_angle),
                b(Cone) + 2.0 * PI * 7.0 * round_section(foot_angle),
            ),
        ),
        case(
            "cone top fillet",
            Cone,
            fillet(0.5, EdgeSelection::Near(vec![[0.0, -2.0, 13.0]]), false),
            Between(
                b(Cone) - 2.0 * PI * 2.0 * 0.25 * round_section(foot_angle),
                b(Cone) - 2.0 * PI * 1.5 * 0.25 * round_section(foot_angle),
            ),
        ),
        case(
            "tangent chain fillet",
            RoundedRect,
            fillet(2.0, top_loop(), true),
            Volume(b(RoundedRect) - swept(4.0 * Q, 2.0 * qx(), 68.0, 2.0 * PI, 4.0, true)),
        ),
        case(
            "tangent chain chamfer",
            RoundedRect,
            chamfer(2.0, top_loop(), true),
            Volume(b(RoundedRect) - swept(2.0, 2.0 / 3.0, 68.0, 2.0 * PI, 4.0, true)),
        ),
        case(
            "4 degree kink fillet",
            Kink,
            fillet(2.0, picked(&[([20.0, 0.0, 5.0], Z)]), false),
            Volume(b(Kink) - 10.0 * 4.0 * round_section(kink_open)),
        ),
        case(
            "4 degree kink chamfer",
            Kink,
            chamfer(2.0, picked(&[([20.0, 0.0, 5.0], Z)]), false),
            Volume(b(Kink) - 10.0 * 2.0 * kink_open.sin()),
        ),
        // A 4 degree turn is no tangent: the chain stops at the kink.
        case(
            "tangent chain stops at a 4 degree kink",
            Kink,
            fillet(1.0, picked(&[([10.0, 0.0, 10.0], X)]), true),
            Volume(b(Kink) - 20.0 * Q),
        ),
        case(
            "radius larger than the side face",
            Thin,
            fillet(3.0, picked(&[([10.0, 0.0, 2.0], X)]), false),
            Refused,
        ),
    ]
}

/// Build `base` and `op` after it.
fn dressed(
    base: Base,
    op: SolidOp,
) -> Result<kernel_api::SolidBuildResult, kernel_api::ChainError> {
    let mut chain = ops(base);
    chain.push(op);
    try_chain(&chain)
}

#[test]
fn every_case_comes_to_its_volume() {
    let mut wrong = Vec::new();
    for Case {
        name,
        base,
        op,
        want,
    } in cases()
    {
        let built = dressed(base, op);
        let verdict = match (&want, built) {
            (Want::Refused, Err(_)) => None,
            (Want::Refused, Ok(_)) => Some("built where no ball seats".to_string()),
            (_, Err(e)) => Some(format!("refused: {}", e.message)),
            (want, Ok(result)) => {
                let v = volume(&result.brep_blob);
                let fits = match want {
                    Want::Volume(w) => (v - w).abs() <= w * 1e-6,
                    Want::Between(lo, hi) => *lo < v && v < *hi,
                    Want::Refused => unreachable!(),
                };
                match (fits, is_sound(&result.brep_blob)) {
                    (true, true) => None,
                    (false, _) => Some(format!("volume {v}")),
                    (true, false) => Some("unsound".to_string()),
                }
            }
        };
        if let Some(why) = verdict {
            wrong.push(format!("{name}: {why}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

/// The base volumes the cases measure from, each built.
#[test]
fn every_base_comes_to_its_volume() {
    use Base::*;
    for base in [
        Block,
        Thin,
        Boss,
        Cone,
        TangentBoss,
        Pocket,
        Lever,
        Kidney,
        Pyramid,
        RoundedRect,
        Kink,
        Slanted,
    ] {
        let v = volume(&try_chain(&ops(base)).unwrap().brep_blob);
        let w = base_volume(base);
        assert!((v - w).abs() <= w * 1e-6, "{base:?}: {v} against {w}");
    }
}

/// A round on a round: the block's four uprights rounded 3, then its top
/// loop rounded 1 along the tangent chain, which rides over the rounds.
#[test]
fn a_round_on_a_round_follows_the_rounds() {
    let uprights = picked(&[
        ([0.0, 0.0, 5.0], Z),
        ([20.0, 0.0, 5.0], Z),
        ([20.0, 20.0, 5.0], Z),
        ([0.0, 20.0, 5.0], Z),
    ]);
    let mut chain = ops(Base::Block);
    chain.push(fillet(3.0, uprights, false));
    chain.push(fillet(1.0, picked(&[([10.0, 0.0, 10.0], X)]), true));
    let v = volume(&try_chain(&chain).unwrap().brep_blob);
    let prism = (400.0 - (4.0 - PI) * 9.0) * 10.0;
    let want = prism - Q * (56.0 + 2.0 * PI * (3.0 - qx()));
    assert!((v - want).abs() <= want * 1e-6, "{v} against {want}");
}

/// A pyramid's apex rounded all at once closes the corner between its four
/// bands, taking more than the same four edges rounded one by one, whose
/// bands stop at caps; both take something.
#[test]
fn a_pyramid_apex_rounded_at_once_takes_its_corner() {
    let at_once = volume(
        &dressed(Base::Pyramid, fillet(1.0, picked(&apex_edges()), false))
            .unwrap()
            .brep_blob,
    );
    let mut chain = ops(Base::Pyramid);
    for edge in apex_edges() {
        chain.push(fillet(1.0, picked(&[edge]), false));
    }
    let one_by_one = volume(&try_chain(&chain).unwrap().brep_blob);
    assert!(
        at_once < one_by_one && one_by_one < base_volume(Base::Pyramid),
        "{at_once}, {one_by_one}"
    );
}

/// The kidney pocket's floor loop of four tangent arcs rounds to its
/// volume in time: within a few times what its outer arc alone takes,
/// timed beside it, so a runner busy with the rest of the suite slows
/// both alike. A loop that takes many times its arcs' sum fails.
#[test]
fn a_kidney_pocket_floor_rounds_in_time() {
    let timed = |edges| {
        let started = std::time::Instant::now();
        let built = dressed(Base::Kidney, fillet(1.0, edges, false)).unwrap();
        (built, started.elapsed().as_secs_f64())
    };
    let (_, one_arc) = timed(picked(&[([15.0, 20.0, 5.0], X)]));
    let (built, floor) = timed(kidney_floor());
    let v = volume(&built.brep_blob);
    let want = base_volume(Base::Kidney) + Q * (12.0 * PI + 2.0 * PI * (3.0 - qx()));
    assert!((v - want).abs() <= want * 1e-6, "{v} against {want}");
    eprintln!("kidney floor {floor:.3} s, its outer arc {one_arc:.3} s");
    assert!(
        floor < 15.0 * one_arc.max(0.01),
        "{floor} s against {one_arc} s for one arc"
    );
}

/// A cone's rims, at its foot on the plate and at its top, chamfer as they
/// round.
#[test]
fn a_cones_rims_chamfer() {
    for point in [[0.0, -6.0, 5.0], [0.0, -2.0, 13.0]] {
        let built = dressed(
            Base::Cone,
            chamfer(0.5, EdgeSelection::Near(vec![point]), false),
        );
        assert!(
            built.as_ref().is_ok_and(|r| is_sound(&r.brep_blob)),
            "{:?}",
            built.err()
        );
    }
}

/// The four concave edges where the slanted bar meets the flanges round
/// and chamfer, adding each edge's section along its 10 mm.
#[test]
fn a_slanted_bar_rounds_into_its_flanges() {
    let feet = || {
        EdgeSelection::Near(vec![
            [4.0, -5.0, 4.72],
            [4.0, -5.0, 10.72],
            [36.0, -5.0, 16.28],
            [36.0, -5.0, 22.28],
        ])
    };
    let rise = (13.0f64 / 36.0).atan();
    let (wide, narrow) = (PI / 2.0 + rise, PI / 2.0 - rise);
    let round = 2.0 * 10.0 * (round_section(wide) + round_section(narrow));
    let bevel = 2.0 * 10.0 * 0.5 * (wide.sin() + narrow.sin());
    for (op, added) in [
        (fillet(1.0, feet(), false), round),
        (chamfer(1.0, feet(), false), bevel),
    ] {
        let v = volume(&dressed(Base::Slanted, op).unwrap().brep_blob);
        let want = base_volume(Base::Slanted) + added;
        assert!((v - want).abs() <= want * 1e-6, "{v} against {want}");
    }
}

/// Every edge of a 2 mm plate rounded 1.9: the bands across each side
/// overlap, and the request is refused at its op.
#[test]
fn a_thin_plates_overlapping_rounds_are_refused() {
    let refused =
        dressed(Base::Thin, fillet(1.9, EdgeSelection::All, false)).expect_err("the bands overlap");
    assert_eq!(refused.op_index, ops(Base::Thin).len());
}

/// A pad of a closed outline, 6 high.
fn padded(outline: ProfileSegment) -> Vec<SolidOp> {
    vec![prism(
        xy_plane(),
        vec![wire(vec![outline])],
        6.0,
        BooleanOp::NewSolid,
    )]
}

/// An upright drum of radius 10 with a bore of radius 3 across it: the
/// bore's rims are curves in space between two cylinders.
fn bored_drum() -> Vec<SolidOp> {
    let drum = |radius, placement, op| SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius,
            height: 40.0,
            angle_deg: 360.0,
        },
        placement,
        op,
    };
    vec![
        drum(10.0, Placement::default(), BooleanOp::NewSolid),
        drum(
            3.0,
            Placement {
                origin: [0.0, -20.0, 20.0],
                x_axis: [1.0, 0.0, 0.0],
                z_axis: [0.0, 1.0, 0.0],
            },
            BooleanOp::Cut,
        ),
    ]
}

/// The three chamfer spellings at `size`.
fn spellings(size: f64) -> [ChamferSpec; 3] {
    [
        ChamferSpec::EqualDistance { distance: size },
        ChamferSpec::TwoDistances {
            distance1: size,
            distance2: size / 2.0,
        },
        ChamferSpec::DistanceAngle {
            distance: size,
            angle_deg: 30.0,
        },
    ]
}

/// An ellipse's and a B-spline's top edges and a bore's rim on a drum
/// chamfer in every spelling, as they round.
#[test]
fn any_edge_chamfers_in_every_spelling() {
    let ellipse = ProfileSegment::Ellipse {
        center: [0.0, 0.0],
        major: [10.0, 0.0],
        ratio: 0.6,
    };
    let spline = ProfileSegment::BSpline {
        control_points: vec![
            [10.0, 0.0],
            [8.0, 8.0],
            [0.0, 10.0],
            [-9.0, 6.0],
            [-10.0, -4.0],
            [0.0, -9.0],
            [8.0, -7.0],
        ],
        periodic: true,
    };
    let cases = [
        ("ellipse", padded(ellipse), [0.0, -6.0, 6.0]),
        ("spline", padded(spline), [0.0, -9.0, 6.0]),
        ("bore rim", bored_drum(), [0.0, -9.5, 22.5]),
    ];
    let mut refused = Vec::new();
    for (name, base, near) in cases {
        for spec in spellings(1.0) {
            let mut chain = base.clone();
            chain.push(SolidOp::Chamfer {
                spec,
                flip: false,
                edges: EdgeSelection::Near(vec![near]),
                follow_tangent: false,
            });
            match try_chain(&chain) {
                Ok(r) if is_sound(&r.brep_blob) => {}
                other => refused.push(format!(
                    "{name} {spec:?}: {:?}",
                    other.err().map(|e| e.message)
                )),
            }
        }
    }
    assert!(refused.is_empty(), "{refused:#?}");
}

/// An L-bracket's front edge (convex) and its inside corner (concave)
/// rounded together close one corner where they meet, whichever is asked
/// first.
#[test]
#[ignore = "kernel: the corner takes some 24 s a build (ogeom-rs#870)"]
fn a_convex_and_a_concave_round_close_one_corner_in_either_order() {
    let bracket = vec![
        prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 20.0, 20.0)],
            5.0,
            BooleanOp::NewSolid,
        ),
        prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 5.0, 20.0)],
            20.0,
            BooleanOp::Fuse,
        ),
    ];
    let inside = ([5.0, 10.0, 5.0], Y);
    let front = ([12.5, 0.0, 5.0], X);
    let round = |order: [([f64; 3], [f64; 3]); 2]| {
        let mut chain = bracket.clone();
        chain.push(fillet(2.0, picked(&order), false));
        volume(&try_chain(&chain).unwrap().brep_blob)
    };
    let (a, b) = (round([front, inside]), round([inside, front]));
    assert!(
        (a - b).abs() < 1e-6 * a,
        "front first {a}, inside first {b}"
    );
}

/// A boss running tangent into the plate takes its foot round as one
/// tangent chain, convex into concave where the boss's wall meets the
/// plate's side.
#[test]
#[ignore = "kernel: a convex and a concave round meeting at a vertex on a cylinder (ogeom-rs#868)"]
fn a_tangent_bosss_foot_rounds() {
    let built = dressed(
        Base::TangentBoss,
        fillet(1.0, EdgeSelection::Near(vec![[14.0, -6.0, 5.0]]), true),
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let v = volume(&built.brep_blob);
    let base = base_volume(Base::TangentBoss);
    // At most a full outward ring of section Q about the boss's radius 6.
    let most = Q * 2.0 * PI * (6.0 + qx());
    assert!(base < v && v < base + most, "volume {v}");
    assert!(is_sound(&built.brep_blob));
}
