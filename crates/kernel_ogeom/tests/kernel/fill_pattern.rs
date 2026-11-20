//! Fill patterns: a feature's tool copied over a sketch's area or a flat
//! face in a grid or a honeycomb and put in by one boolean, measured
//! against the closed forms of plates with holes in them.

use std::f64::consts::PI;

use kernel_api::{
    BooleanOp, FaceProbe, FillLattice, FillLayout, FillRegion, PatternStep, Placement,
    PrimitiveKind, Profile, SolidOp,
};

use super::common::{chain, is_sound, plane_at_z, primitive, rect_wire, try_chain, volume};

/// A plate `w` × `h`, 2 thick, on the XY plane.
fn plate(w: f64, h: f64) -> SolidOp {
    primitive(PrimitiveKind::Box {
        length: w,
        width: h,
        height: 2.0,
    })
}

/// A tool through the plate at `(x, y)`: a cylinder of `radius`, or a
/// square box `radius` each way from its centre; cut or fused.
fn tool(x: f64, y: f64, radius: f64, round: bool, op: BooleanOp) -> SolidOp {
    let (kind, origin) = if round {
        (
            PrimitiveKind::Cylinder {
                radius,
                height: 4.0,
                angle_deg: 360.0,
            },
            [x, y, -1.0],
        )
    } else {
        (
            PrimitiveKind::Box {
                length: 2.0 * radius,
                width: 2.0 * radius,
                height: 4.0,
            },
            [x - radius, y - radius, -1.0],
        )
    };
    SolidOp::Primitive {
        kind,
        placement: Placement {
            origin,
            ..Placement::default()
        },
        op,
    }
}

/// The outline of a tool `radius` each way about `(x, y)`.
fn square_outline(x: f64, y: f64, radius: f64) -> Vec<[f64; 3]> {
    [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
        .iter()
        .map(|[a, b]| [x + a * radius, y + b * radius, 2.0])
        .collect()
}

fn fill(region: FillRegion, lattice: FillLattice, anchor: [f64; 3]) -> FillLayout {
    FillLayout {
        region,
        lattice,
        angle_deg: 0.0,
        margin: 0.0,
        clip: false,
        anchor,
        outline: Vec::new(),
    }
}

fn pattern(layout: FillLayout, skipped: Vec<u32>) -> SolidOp {
    SolidOp::Pattern {
        steps: vec![PatternStep::Fill(layout)],
        originals: vec![1],
        skipped,
    }
}

/// The plate's top face.
fn top(x: f64, y: f64) -> FillRegion {
    FillRegion::Face(FaceProbe {
        point: [x, y, 2.0],
        normal: [0.0, 0.0, 1.0],
        name: 0,
    })
}

#[track_caller]
fn assert_volume(ops: &[SolidOp], want: f64) {
    let built = chain(ops);
    assert!(is_sound(&built.brep_blob), "a closed solid");
    let got = volume(&built.brep_blob);
    assert!(
        (got - want).abs() < 1e-3 * want.max(1.0),
        "{got}, want {want}"
    );
}

/// Holes 10 apart over a 40 × 30 plate's top face, the first at (5, 5):
/// twelve holes in all, the plate less 12 · π r² · t; one skipped, eleven.
#[test]
fn a_grid_over_a_face_cuts_every_hole_in_one_go() {
    let r: f64 = 1.5;
    let ops = |skipped| {
        vec![
            plate(40.0, 30.0),
            tool(5.0, 5.0, r, true, BooleanOp::Cut),
            pattern(
                fill(
                    top(20.0, 15.0),
                    FillLattice::Grid { x: 10.0, y: 10.0 },
                    [5.0, 5.0, 2.0],
                ),
                skipped,
            ),
        ]
    };
    let hole = PI * r * r * 2.0;
    assert_volume(&ops(Vec::new()), 40.0 * 30.0 * 2.0 - 12.0 * hole);
    assert_volume(&ops(vec![3]), 40.0 * 30.0 * 2.0 - 11.0 * hole);
}

/// Bosses fused over a sketch's square, a honeycomb 6 apart: the plate
/// plus each boss's 2 mm standing above it.
#[test]
fn a_honeycomb_of_bosses_fuses_onto_the_plate() {
    let r: f64 = 1.0;
    let mut layout = fill(
        FillRegion::Profile(Profile {
            plane: plane_at_z(2.0),
            wires: vec![rect_wire(0.0, 0.0, 30.0, 30.0)],
        }),
        FillLattice::Honeycomb { pitch: 6.0 },
        [3.0, 3.0, 2.0],
    );
    layout.outline = square_outline(3.0, 3.0, r);
    let built = chain(&[
        plate(30.0, 30.0),
        tool(3.0, 3.0, r, true, BooleanOp::Fuse),
        pattern(layout.clone(), Vec::new()),
    ]);
    assert!(is_sound(&built.brep_blob), "a closed solid");
    // Rows 6·√3/2 apart from y = 3: six rows; the even rows hold 3, 9,
    // .. 27 (five), the odd rows 6, .. 24 (four), each whole inside; each
    // boss stands 1 above the plate and 1 below.
    let count = 3.0 * 5.0 + 3.0 * 4.0;
    let want = 30.0 * 30.0 * 2.0 + count * PI * r * r * 2.0;
    let got = volume(&built.brep_blob);
    assert!((got - want).abs() < 1e-3 * want, "{got}, want {want}");
}

/// Square holes 4 wide, 8 apart from (10, 10), over a sketch's square
/// from 3 to 17: kept whole, the middle one alone (the original); trimmed
/// at the edge, those at 2 and 18 keep a 1 mm strip inside, so the holes
/// cover (1 + 4 + 1)² of the plate; with a 0.5 margin, (0.5 + 4 + 0.5)².
#[test]
fn copies_the_edge_passes_through_are_left_out_or_trimmed_to_the_region() {
    let region = || {
        FillRegion::Profile(Profile {
            plane: plane_at_z(2.0),
            wires: vec![rect_wire(3.0, 3.0, 17.0, 17.0)],
        })
    };
    let ops = |clip: bool, margin: f64| {
        let mut layout = fill(
            region(),
            FillLattice::Grid { x: 8.0, y: 8.0 },
            [10.0, 10.0, 2.0],
        );
        layout.outline = square_outline(10.0, 10.0, 2.0);
        layout.clip = clip;
        layout.margin = margin;
        vec![
            plate(30.0, 30.0),
            tool(10.0, 10.0, 2.0, false, BooleanOp::Cut),
            pattern(layout, Vec::new()),
        ]
    };
    let plate_volume = 30.0 * 30.0 * 2.0;
    assert_volume(&ops(false, 0.0), plate_volume - 16.0 * 2.0);
    assert_volume(&ops(true, 0.0), plate_volume - 36.0 * 2.0);
    assert_volume(&ops(true, 0.5), plate_volume - 25.0 * 2.0);
}

/// Holes whose copies overlap their neighbours cut as one slot: circles
/// of radius 1, 1.5 apart, centred from 0.5 to 18.5 along a strip; the
/// plate loses each circle less the lens it shares with the next, and
/// the first circle's cap past the plate's end.
#[test]
fn overlapping_copies_cut_as_one() {
    let layout = fill(
        FillRegion::Profile(Profile {
            plane: plane_at_z(2.0),
            wires: vec![rect_wire(0.2, 4.0, 19.8, 6.0)],
        }),
        FillLattice::Grid { x: 1.5, y: 10.0 },
        [5.0, 5.0, 2.0],
    );
    let (r, d): (f64, f64) = (1.0, 1.5);
    let lens = 2.0 * r * r * (d / (2.0 * r)).acos() - d / 2.0 * (4.0 * r * r - d * d).sqrt();
    let cap = r * r * (0.5f64).acos() - 0.5 * (r * r - 0.25).sqrt();
    let slot = 13.0 * PI * r * r - 12.0 * lens - cap;
    assert_volume(
        &[
            plate(20.0, 10.0),
            tool(5.0, 5.0, r, true, BooleanOp::Cut),
            pattern(layout, Vec::new()),
        ],
        20.0 * 10.0 * 2.0 - slot * 2.0,
    );
}

/// A fill whose face is curved, or whose copies pass the limit, fails on
/// the pattern's op by name.
#[test]
fn a_fill_that_cannot_be_made_fails_by_name() {
    let curved = vec![
        primitive(PrimitiveKind::Cylinder {
            radius: 10.0,
            height: 10.0,
            angle_deg: 360.0,
        }),
        tool(10.0, 0.0, 1.0, true, BooleanOp::Cut),
        pattern(
            fill(
                FillRegion::Face(FaceProbe {
                    point: [10.0, 0.0, 5.0],
                    normal: [1.0, 0.0, 0.0],
                    name: 0,
                }),
                FillLattice::Grid { x: 3.0, y: 3.0 },
                [10.0, 0.0, 5.0],
            ),
            Vec::new(),
        ),
    ];
    let err = try_chain(&curved).unwrap_err();
    assert_eq!(err.op_index, 2);
    assert!(err.message.contains("not flat"), "{}", err.message);
    let many = vec![
        plate(500.0, 500.0),
        tool(5.0, 5.0, 1.0, true, BooleanOp::Cut),
        pattern(
            fill(
                top(250.0, 250.0),
                FillLattice::Grid { x: 5.0, y: 5.0 },
                [5.0, 5.0, 2.0],
            ),
            Vec::new(),
        ),
    ];
    let err = try_chain(&many).unwrap_err();
    assert!(err.message.contains("2000"), "{}", err.message);
}

/// The kernel's places for a fill over a face, asked of the solid it
/// built, match the copies it cut: the original and each copy.
#[test]
fn a_fills_places_are_asked_of_the_solid_it_built() {
    use kernel_api::KernelQueries;
    let layout = fill(
        top(20.0, 15.0),
        FillLattice::Grid { x: 10.0, y: 10.0 },
        [5.0, 5.0, 2.0],
    );
    let built = chain(&[
        plate(40.0, 30.0),
        tool(5.0, 5.0, 1.0, true, BooleanOp::Cut),
        pattern(layout.clone(), Vec::new()),
    ]);
    let places = kernel_ogeom::QUERIES
        .pattern_transforms(Some(&built.brep_blob), &[PatternStep::Fill(layout)])
        .unwrap();
    assert_eq!(places.len(), 12);
    assert_eq!(places[1][0][3], 10.0);
}

/// Through the Design bench: a honeycomb of holes 8 apart over a plate's
/// top face, kept 1 mm inside its edge: eighteen holes (the original
/// among them) whose rims stay 1 mm clear of every side.
#[test]
fn a_fill_feature_vents_a_plate_in_a_honeycomb() {
    use super::common::{Features, body_volume, primitive_feature, registry, settle_as_host};
    use core_document::Document;
    use wb_design::{DesignFeature, FacePick, FillArea, FillKind};
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Plate".into()));
    doc.add_in(
        body,
        "Plate",
        primitive_feature(
            PrimitiveKind::Box {
                length: 40.0,
                width: 30.0,
                height: 2.0,
            },
            [0.0; 3],
        ),
    );
    let mut vent = primitive_feature(
        PrimitiveKind::Cylinder {
            radius: 1.5,
            height: 4.0,
            angle_deg: 360.0,
        },
        [5.0, 5.0, -1.0],
    );
    if let DesignFeature::Primitive { subtractive, .. } = &mut vent {
        *subtractive = true;
    }
    let vent = doc.add_in(body, "Vent", vent);
    let fill = doc.add_in(
        body,
        "Fill",
        DesignFeature::FillPattern {
            originals: vec![vent],
            region: FillArea::Face(FacePick {
                point: [20.0, 15.0, 2.0],
                normal: [0.0, 0.0, 1.0],
                name: 0,
            }),
            lattice: FillKind::Honeycomb,
            spacing: 8.0,
            row_spacing: 8.0,
            angle_deg: 0.0,
            margin: 1.0,
            clip: false,
            refine: false,
            boolean_tolerance: None,
            skipped: Vec::new(),
        },
    );
    doc.mark_feature_dirty(fill);
    settle_as_host(&registry, &mut doc);
    let error = &doc.get_feature_meta(fill).unwrap().error;
    assert!(error.is_none(), "it builds: {error:?}");
    // Rows at y = 5, 11.9, 18.9 and 25.8; the even ones at x = 5 to 37,
    // five holes, the odd ones at 9 to 33, four.
    let want = 40.0 * 30.0 * 2.0 - 18.0 * PI * 1.5 * 1.5 * 2.0;
    let got = body_volume(&doc, body);
    assert!((got - want).abs() < 1e-3 * want, "{got}, want {want}");
}

/// Big fills, past the default program's budget: 1800 holes in a plate
/// cut in one boolean, against its closed form.
#[cfg(feature = "heavy-tests")]
mod heavy {
    use super::*;

    #[test]
    fn eighteen_hundred_holes_cut_in_one_boolean() {
        let r: f64 = 0.8;
        let started = std::time::Instant::now();
        let ops = vec![
            plate(112.5, 100.0),
            tool(1.25, 1.25, r, true, BooleanOp::Cut),
            pattern(
                fill(
                    top(56.25, 50.0),
                    FillLattice::Grid { x: 2.5, y: 2.5 },
                    [1.25, 1.25, 2.0],
                ),
                Vec::new(),
            ),
        ];
        assert_volume(&ops, 112.5 * 100.0 * 2.0 - 1800.0 * PI * r * r * 2.0);
        eprintln!("1800 holes in {:?}", started.elapsed());
    }
}
