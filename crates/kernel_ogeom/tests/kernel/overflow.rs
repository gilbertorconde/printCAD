//! What a fillet or chamfer does where it reaches geometry other than its
//! own two faces (`kernel_api::Overflow`): a boss's foot round blending
//! into the block's edge round, a sharp edge held, a round trimming
//! through another face, a narrow slot rounded through; each switched off
//! falls to the next or is refused at its op.

use std::f64::consts::{PI, TAU};

use kernel_api::{
    BooleanOp, ChamferSpec, EdgeSelection, ExtrudeTermination, Overflow, Profile, ProfilePlane,
    ProfileWire, SolidOp, SweepKind,
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

fn fillet(radius: f64, near: &[[f64; 3]], overflow: Overflow) -> SolidOp {
    SolidOp::Fillet {
        radius,
        edges: EdgeSelection::Near(near.to_vec()),
        follow_tangent: true,
        corner: Default::default(),
        distances: None,
        auto: false,
        variable: None,
        overflow,
    }
}

fn chamfer(distance: f64, near: &[[f64; 3]], overflow: Overflow) -> SolidOp {
    SolidOp::Chamfer {
        spec: ChamferSpec::EqualDistance { distance },
        flip: false,
        edges: EdgeSelection::Near(near.to_vec()),
        follow_tangent: true,
        corner: Default::default(),
        overflow,
    }
}

/// Every behaviour off.
const NONE: Overflow = Overflow {
    blend_into_round: false,
    hold_edge: false,
    hold_edge_at_end: false,
    trim_through: false,
    narrow_blend_made: false,
    narrow_whole_face: false,
    narrow_partial_face: false,
};

fn with(change: impl FnOnce(&mut Overflow)) -> Overflow {
    let mut overflow = Overflow::default();
    change(&mut overflow);
    overflow
}

/// The volume `ops` build, which must be one sound solid.
#[track_caller]
fn built(ops: &[SolidOp]) -> f64 {
    let result = try_chain(ops).unwrap_or_else(|e| panic!("builds: {e}"));
    assert!(is_sound(&result.brep_blob), "a sound solid");
    volume(&result.brep_blob)
}

/// Why the last of `ops` is refused, at that op.
#[track_caller]
fn refused(ops: &[SolidOp]) -> String {
    let err = try_chain(ops).expect_err("refused");
    assert_eq!(err.op_index, ops.len() - 1, "{}", err.message);
    err.message
}

/// The block 40 x 30 x 15, its top front edge (along `y = 0`) rounded at 5,
/// and a boss of radius 6 standing 15 tall on its top about `(20, 12)`:
/// the foot round's ball, 3 across, reaches 2 past the edge round's
/// tangent line `y = 5`.
fn bossed() -> Vec<SolidOp> {
    vec![
        prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 40.0, 30.0)],
            15.0,
            BooleanOp::NewSolid,
        ),
        fillet(5.0, &[[20.0, 0.0, 15.0]], Overflow::default()),
        prism(
            plane_at_z(15.0),
            vec![circle_wire(20.0, 12.0, 6.0)],
            15.0,
            BooleanOp::Fuse,
        ),
    ]
}

/// A point on the boss's foot rim.
const FOOT: [f64; 3] = [26.0, 12.0, 15.0];

/// The fill a foot round of radius 3 adds about a boss of radius 6 on a
/// flat floor (Pappus: the spandrel's area `9 (1 - pi/4)` swept about the
/// axis at its centroid's radius).
fn torus_fill() -> f64 {
    TAU * (6.0 + 3.0 * (10.0 - 3.0 * PI) / (12.0 - 3.0 * PI)) * 9.0 * (1.0 - PI / 4.0)
}

/// The owner's case: the boss's foot round meets the block's edge round.
/// Blend into round (on by default) bends it over the edge round into one
/// sound solid; its fill is the torus's but over the ring's stretch past
/// the tangent line, where the floor falls away under it. Off, the request
/// goes on to trim through, which refuses it as before, and so does every
/// behaviour off.
#[test]
fn a_boss_s_foot_round_blends_into_the_edge_round_it_reaches() {
    let base = bossed();
    let before = built(&base);
    let foot = |overflow| [base.clone(), vec![fillet(3.0, &[FOOT], overflow)]].concat();

    let fill = built(&foot(Overflow::default())) - before;
    // The ring past the tangent line `y = 5` spans the angle where the
    // ball's reach (9 from the axis) crosses it, 7 from the axis; the
    // floor there falls at most `5 - sqrt(21)` below the top.
    let half = (7.0_f64 / 9.0).acos();
    let past = half / PI * torus_fill();
    let fall = 5.0 - 21.0_f64.sqrt();
    let ring = half * (81.0 - 36.0);
    assert!(
        fill > torus_fill() - past && fill < torus_fill() + ring * fall,
        "the fill {fill} against the torus's {}",
        torus_fill()
    );
    assert!(
        (fill - torus_fill()).abs() > 1e-2,
        "bent over the edge round, not the torus's: {fill}"
    );

    let off = refused(&foot(with(|o| o.blend_into_round = false)));
    assert!(off.contains("does not reach so far"), "{off}");
    refused(&foot(NONE));
    // Trim through off alone: blend into round still wins.
    let trim_off = built(&foot(with(|o| o.trim_through = false))) - before;
    assert!((trim_off - fill).abs() < 1e-6, "{trim_off} against {fill}");
}

/// Rolling-ball corners take the switches too: the foot round with blend
/// into round off is refused as it is with setback corners.
#[test]
fn a_rolling_ball_foot_round_takes_the_overflow_switches() {
    let mut op = fillet(3.0, &[FOOT], with(|o| o.blend_into_round = false));
    if let SolidOp::Fillet { corner, .. } = &mut op {
        *corner = kernel_api::FilletCorner::RollingBall;
    }
    let why = refused(&[bossed(), vec![op]].concat());
    assert!(why.contains("does not reach so far"), "{why}");
}

/// A variable round holding one radius and a round of two equal distances
/// are the round of that radius, and take its switches: by default each
/// blends the foot into the edge round as the round of 3 does, and with
/// blend into round off each is refused.
#[test]
fn one_radius_rounds_of_a_law_or_two_distances_take_the_overflow_switches() {
    let base = bossed();
    let round = built(
        &[
            base.clone(),
            vec![fillet(3.0, &[FOOT], Overflow::default())],
        ]
        .concat(),
    );
    let law = |overflow| {
        let mut op = fillet(3.0, &[FOOT], overflow);
        if let SolidOp::Fillet { variable, .. } = &mut op {
            *variable = Some(kernel_api::VariableRadius {
                end: 3.0,
                between: Vec::new(),
                reversed: false,
            });
        }
        op
    };
    let distances = |overflow| {
        let mut op = fillet(3.0, &[FOOT], overflow);
        if let SolidOp::Fillet { distances, .. } = &mut op {
            *distances = Some(kernel_api::RoundDistances {
                other: 3.0,
                flip: false,
                profile: kernel_api::RoundProfile::Elliptical,
            });
        }
        op
    };
    let off = with(|o| o.blend_into_round = false);
    for (what, op) in [
        ("law", law as fn(Overflow) -> SolidOp),
        ("distances", distances),
    ] {
        let blended = built(&[base.clone(), vec![op(Overflow::default())]].concat());
        assert!(
            (blended - round).abs() < 1e-6,
            "{what}: {blended} against {round}"
        );
        let why = refused(&[base.clone(), vec![op(off)]].concat());
        assert!(why.contains("does not reach so far"), "{what}: {why}");
    }
}

/// A chamfer takes blend into round too: the boss's foot bevel of 3 runs
/// on over the edge round. Off, it trims through as it did before: the
/// whole cone's fill, `2 pi 7 4.5`, its floor ring carried on flat over
/// the edge round.
#[test]
fn a_boss_s_foot_bevel_blends_into_the_edge_round_or_trims_through() {
    let base = bossed();
    let before = built(&base);
    let foot = |overflow| [base.clone(), vec![chamfer(3.0, &[FOOT], overflow)]].concat();
    let cone = TAU * 7.0 * 4.5;
    let blended = built(&foot(Overflow::default())) - before;
    assert!(
        (blended - cone).abs() > 1e-2 && blended < cone,
        "the bevel runs down over the round: {blended} against the cone's {cone}"
    );
    let trimmed = built(&foot(with(|o| o.blend_into_round = false))) - before;
    assert!(
        (trimmed - cone).abs() < 1e-6,
        "the cone's fill {trimmed} against {cone}"
    );
}

/// Hold edge: the block 30 x 20 x 15's top front edge rounded at 4 beside a
/// pocket of radius 5 about `(15, 7)`, whose rim comes within 2 of the
/// edge. By default the ball rests on the rim where its contact would
/// fall inside it, and the rim is kept; with hold edge off the round runs
/// on through the pocket's wall, cutting the rim away: more material goes.
/// With trim through off too it is refused by name.
#[test]
fn hold_edge_keeps_a_pocket_rim_the_round_would_run_past() {
    let base = vec![
        prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 30.0, 20.0)],
            15.0,
            BooleanOp::NewSolid,
        ),
        prism(
            plane_at_z(7.0),
            vec![circle_wire(15.0, 7.0, 5.0)],
            8.0,
            BooleanOp::Cut,
        ),
    ];
    let round = |overflow| {
        [
            base.clone(),
            vec![fillet(4.0, &[[5.0, 0.0, 15.0]], overflow)],
        ]
        .concat()
    };
    let held = built(&round(Overflow::default()));
    let trimmed = built(&round(with(|o| o.hold_edge = false)));
    // Away from the pocket the band takes `16 - 4 pi` of section.
    let pocketed = built(&base);
    assert!(held < pocketed && held > pocketed - 30.0 * (16.0 - 4.0 * PI));
    assert!(trimmed < held - 1e-3, "{trimmed} against {held}");
    let why = refused(&round(with(|o| {
        o.hold_edge = false;
        o.trim_through = false;
    })));
    assert!(why.contains("hold edge is off"), "{why}");
}

/// A plate's boss of radius 5, a small pocket sunk into its top 0.2 in
/// from the rim.
fn pocketed_boss() -> Vec<SolidOp> {
    vec![
        prism(
            xy_plane(),
            vec![rect_wire(-20.0, -20.0, 20.0, 20.0)],
            4.0,
            BooleanOp::NewSolid,
        ),
        prism(
            plane_at_z(4.0),
            vec![circle_wire(0.0, 0.0, 5.0)],
            5.0,
            BooleanOp::Fuse,
        ),
        prism(
            plane_at_z(6.0),
            vec![rect_wire(4.3, -0.25, 4.8, 0.25)],
            4.0,
            BooleanOp::Cut,
        ),
    ]
}

/// The boss's top rim rounded at 1 reaches the pocket's walls and trims
/// through them; an edge reaching nothing else rounds exactly with every
/// behaviour off.
#[test]
fn trim_through_rounds_on_through_another_face() {
    let rim = [
        pocketed_boss(),
        vec![fillet(1.0, &[[-5.0, 0.0, 9.0]], Overflow::default())],
    ];
    built(&rim.concat());

    let block = prism(
        xy_plane(),
        vec![rect_wire(0.0, 0.0, 10.0, 8.0)],
        6.0,
        BooleanOp::NewSolid,
    );
    let local = built(&[block, fillet(2.0, &[[5.0, 0.0, 6.0]], NONE)]);
    let want = 480.0 - 10.0 * 4.0 * (1.0 - PI / 4.0);
    assert!((local - want).abs() < 1e-6, "{local} against {want}");
}

/// With trim through off the rim's round reaching the pocket is refused
/// by name at its op.
#[test]
fn trim_through_off_refuses_a_round_reaching_another_face() {
    let rim = [
        pocketed_boss(),
        vec![fillet(
            1.0,
            &[[-5.0, 0.0, 9.0]],
            with(|o| o.trim_through = false),
        )],
    ];
    let why = refused(&rim.concat());
    assert!(why.contains("trim through is off"), "{why}");
}

/// A narrow passage: a slot 6 wide, its floor's two rims rounded at 4, more
/// than half its width. Each round keeps its section and runs on until the
/// two meet mid-slot (closed form: the two quarter circles' spandrels cut
/// where they cross). With every narrow-passage strategy off it is refused
/// by name.
#[test]
fn a_slot_narrower_than_twice_the_radius_is_rounded_through() {
    let ops = vec![
        prism(
            xy_plane(),
            vec![rect_wire(0.0, 0.0, 20.0, 30.0)],
            15.0,
            BooleanOp::NewSolid,
        ),
        prism(
            plane_at_z(7.0),
            vec![rect_wire(-1.0, 10.0, 21.0, 16.0)],
            9.0,
            BooleanOp::Cut,
        ),
    ];
    let rims = [[10.0, 10.0, 7.0], [10.0, 16.0, 7.0]];
    let rounded = |overflow| {
        let mut op = fillet(4.0, &rims, overflow);
        if let SolidOp::Fillet { follow_tangent, .. } = &mut op {
            *follow_tangent = false;
        }
        [ops.clone(), vec![op]].concat()
    };
    let before = built(&ops);
    let added = built(&rounded(Overflow::default())) - before;
    let (r, w) = (4.0_f64, 6.0_f64);
    let g = |u: f64| (u * (r * r - u * u).max(0.0).sqrt() + r * r * (u / r).asin()) / 2.0;
    let area = 2.0 * (r * w / 2.0 - (g(r) - g(r - w / 2.0)));
    assert!(
        (added - area * 20.0).abs() < 1e-6,
        "{added} against {}",
        area * 20.0
    );
    let why = refused(&rounded(with(|o| {
        o.narrow_blend_made = false;
        o.narrow_whole_face = false;
        o.narrow_partial_face = false;
    })));
    assert!(
        why.contains("every narrow-passage strategy is off"),
        "{why}"
    );
}

/// What needs a long build, compiled and run only with the `heavy-tests`
/// feature (`node scripts/test-budget.mjs --heavy`): each limit reading
/// builds the round tens of times.
#[cfg(feature = "heavy-tests")]
mod heavy {
    use super::*;

    /// The largest radius a drag stops at, as a preview of the foot round at
    /// 0.5 under `overflow` reads it.
    fn foot_limit(overflow: Overflow) -> f64 {
        let ops = [bossed(), vec![fillet(0.5, &[FOOT], overflow)]].concat();
        let detail = kernel_api::TessellationSettings::default();
        let previewed = super::super::common::new_kernel()
            .execute_solid_chain(kernel_ogeom::ChainRequest::new(&ops, &detail).preview(Some(3..4)))
            .expect("half a millimetre builds");
        let limit = previewed.size_limit.expect("the preview's limit");
        limit.largest.expect("a radius that fails")
    }

    /// A drag stops where the build under the feature's own options stops:
    /// with blend into round off the foot round is refused at 3, so its limit
    /// is below 3, where it builds; by default it builds at 3 and the limit
    /// lies beyond.
    #[test]
    fn the_drag_limit_reads_the_feature_s_overflow_options() {
        let base = bossed();
        let off = with(|o| o.blend_into_round = false);
        let at =
            |radius, overflow| [base.clone(), vec![fillet(radius, &[FOOT], overflow)]].concat();
        let limit = foot_limit(off);
        assert!(limit < 3.0, "{limit}");
        built(&at(limit, off));
        assert!(foot_limit(Overflow::default()) > 3.0);
    }

    /// The limit under options other than the default is the kernel's
    /// reading: the build holds at it and is refused a ten-thousandth
    /// above it.
    #[test]
    fn the_drag_limit_under_overflow_options_is_exact() {
        let base = bossed();
        let off = with(|o| o.blend_into_round = false);
        let at =
            |radius, overflow| [base.clone(), vec![fillet(radius, &[FOOT], overflow)]].concat();
        let limit = foot_limit(off);
        built(&at(limit, off));
        refused(&at(limit * (1.0 + 1e-4), off));
    }

    /// The same for rolling-ball corners, whose rounds take the options:
    /// with blend into round off the foot round's limit is the largest
    /// radius that builds under them.
    #[test]
    #[ignore = "kernel: no limit reading takes the overflow options of rolling-ball corners (ogeom-rs#1236)"]
    fn the_rolling_ball_drag_limit_under_overflow_options_is_exact() {
        let base = bossed();
        let off = with(|o| o.blend_into_round = false);
        let rolling = |radius, overflow| {
            let mut op = fillet(radius, &[FOOT], overflow);
            if let SolidOp::Fillet { corner, .. } = &mut op {
                *corner = kernel_api::FilletCorner::RollingBall;
            }
            [base.clone(), vec![op]].concat()
        };
        let detail = kernel_api::TessellationSettings::default();
        let ops = rolling(0.5, off);
        let limit = super::super::common::new_kernel()
            .execute_solid_chain(kernel_ogeom::ChainRequest::new(&ops, &detail).preview(Some(3..4)))
            .expect("half a millimetre builds")
            .size_limit
            .and_then(|limit| limit.largest)
            .expect("a radius that fails");
        assert!(limit < 3.0, "{limit}");
        built(&rolling(limit, off));
        refused(&rolling(limit * (1.0 + 1e-4), off));
    }

    /// The same for a variable round holding one radius, which takes every
    /// switch as the round of that radius does: with blend into round off
    /// its radius, the closed rim's start and end at once, stops at the
    /// largest that builds under them.
    #[test]
    #[ignore = "kernel: no limit reading of a radius law takes the overflow options (ogeom-rs#1236)"]
    fn a_variable_round_s_drag_limit_under_overflow_options_is_exact() {
        let base = bossed();
        let off = with(|o| o.blend_into_round = false);
        let law = |start, end| {
            let mut op = fillet(start, &[FOOT], off);
            if let SolidOp::Fillet { variable, .. } = &mut op {
                *variable = Some(kernel_api::VariableRadius {
                    end,
                    between: Vec::new(),
                    reversed: false,
                });
            }
            [base.clone(), vec![op]].concat()
        };
        let detail = kernel_api::TessellationSettings::default();
        let ops = law(0.5, 0.5);
        let limit = super::super::common::new_kernel()
            .execute_solid_chain(kernel_ogeom::ChainRequest::new(&ops, &detail).preview(Some(3..4)))
            .expect("half a millimetre builds")
            .size_limit
            .expect("the preview's limit");
        let start = limit
            .law
            .first()
            .and_then(|law| law.largest)
            .expect("a start radius that fails");
        assert!(start < 3.0, "{start}");
        // The rim is closed: its law's start and end are one point.
        built(&law(start, start));
        let past = start * (1.0 + 1e-4);
        refused(&law(past, past));
    }
}
