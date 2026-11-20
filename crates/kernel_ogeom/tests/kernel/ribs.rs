//! Ribs grown from an open sketch line until they meet the solid,
//! measured against the web's closed form.

use kernel_api::{
    BooleanOp, Placement, PrimitiveKind, Profile, ProfileSegment, ProfileWire, RibForm, RibSide,
    SolidOp,
};

use super::common::{chain, is_sound, primitive_feature, try_chain, volume, xy_plane};

/// A U channel on the XY plane: a floor 20 × 20 × 2 and two walls 2
/// thick, 20 tall, at either end of X.
fn channel() -> Vec<SolidOp> {
    channel_of(20.0)
}

/// [`channel`] with walls `wall` tall.
fn channel_of(wall: f64) -> Vec<SolidOp> {
    let slab = |x: f64, y: f64, z: f64, (l, w, h): (f64, f64, f64)| SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: l,
            width: w,
            height: h,
        },
        placement: Placement {
            origin: [x, y, z],
            ..Placement::default()
        },
        op: BooleanOp::Fuse,
    };
    let mut ops = vec![slab(0.0, 0.0, 0.0, (20.0, 20.0, 2.0))];
    if let SolidOp::Primitive { op, .. } = &mut ops[0] {
        *op = BooleanOp::NewSolid;
    }
    ops.push(slab(0.0, 0.0, 0.0, (2.0, 20.0, wall)));
    ops.push(slab(18.0, 0.0, 0.0, (2.0, 20.0, wall)));
    ops
}

const CHANNEL: f64 = 20.0 * 20.0 * 2.0 + 2.0 * 2.0 * 20.0 * 18.0;

/// A plane across the channel at y = 10, its X the channel's X and its
/// Y the channel's Z.
fn across() -> kernel_api::ProfilePlane {
    kernel_api::ProfilePlane {
        origin: [0.0, 10.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 0.0, 1.0],
        normal: [0.0, -1.0, 0.0],
    }
}

fn rib(plane: kernel_api::ProfilePlane, segments: Vec<ProfileSegment>, form: RibForm) -> SolidOp {
    SolidOp::Rib {
        profile: Profile {
            plane,
            wires: vec![ProfileWire::new(segments)],
        },
        form,
    }
}

/// A line across the channel 12 mm up, short of both walls: across the
/// plane its ends run on to the walls and the web fills down to the floor,
/// the side the channel closes in; above it nothing stops it.
#[test]
fn a_rib_across_its_plane_fills_between_its_line_and_the_solid() {
    for (side, thickness) in [(RibSide::Both, 2.0), (RibSide::Front, 3.0)] {
        let mut ops = channel();
        ops.push(rib(
            across(),
            vec![ProfileSegment::Line {
                start: [6.0, 12.0],
                end: [14.0, 12.0],
            }],
            RibForm {
                thickness,
                side,
                along_normal: false,
            },
        ));
        let built = chain(&ops);
        assert!(is_sound(&built.brep_blob), "a closed solid");
        let web = 16.0 * 10.0 * thickness;
        let got = volume(&built.brep_blob);
        assert!(
            (got - (CHANNEL + web)).abs() < 1e-3,
            "{side:?}: {got}, want {}",
            CHANNEL + web
        );
    }
}

/// A line 12 mm up across a channel whose walls stand 8 tall: its ends
/// run on over the walls, and the web fills down to the floor between
/// them and onto their tops, cut off square where the walls end along the
/// line rather than leaking round them.
#[test]
fn a_rib_over_short_walls_stops_where_they_end() {
    let thickness = 2.0;
    let mut ops = channel_of(8.0);
    ops.push(rib(
        across(),
        vec![ProfileSegment::Line {
            start: [6.0, 12.0],
            end: [14.0, 12.0],
        }],
        RibForm {
            thickness,
            side: RibSide::Both,
            along_normal: false,
        },
    ));
    let built = chain(&ops);
    assert!(is_sound(&built.brep_blob), "a closed solid");
    let channel = 20.0 * 20.0 * 2.0 + 2.0 * 2.0 * 20.0 * 6.0;
    // Between the walls from the floor up to the line, and over each
    // wall's top up to it.
    let web = (16.0 * 10.0 + 2.0 * 2.0 * 4.0) * thickness;
    let got = volume(&built.brep_blob);
    assert!(
        (got - (channel + web)).abs() < 1e-3,
        "{got}, want {}",
        channel + web
    );
}

/// A line on a plane above the floor, along the normal: the strip it
/// makes runs down to the floor; up, nothing stops it, so it goes only
/// down.
#[test]
fn a_rib_along_the_normal_runs_down_to_the_floor() {
    let mut ops = channel();
    ops.push(rib(
        kernel_api::ProfilePlane {
            origin: [0.0, 0.0, 12.0],
            ..xy_plane()
        },
        vec![ProfileSegment::Line {
            start: [6.0, 4.0],
            end: [6.0, 16.0],
        }],
        RibForm {
            thickness: 2.0,
            side: RibSide::Both,
            along_normal: true,
        },
    ));
    let built = chain(&ops);
    assert!(is_sound(&built.brep_blob), "a closed solid");
    let web = 12.0 * 10.0 * 2.0;
    let got = volume(&built.brep_blob);
    assert!(
        (got - (CHANNEL + web)).abs() < 1e-3,
        "{got}, want {}",
        CHANNEL + web
    );
}

/// A line that meets nothing, or one that closes on itself, fails on the
/// rib's op.
#[test]
fn a_rib_that_meets_no_solid_or_closes_fails() {
    let line = |start, end| ProfileSegment::Line { start, end };
    let form = RibForm {
        thickness: 2.0,
        ..Default::default()
    };
    let mut far = channel();
    far.push(rib(
        kernel_api::ProfilePlane {
            origin: [0.0, 100.0, 0.0],
            ..across()
        },
        vec![line([6.0, 12.0], [14.0, 12.0])],
        form,
    ));
    let err = try_chain(&far).unwrap_err();
    assert!(err.message.contains("meets no solid"), "{}", err.message);
    let mut closed = channel();
    closed.push(rib(
        across(),
        vec![
            line([6.0, 6.0], [14.0, 6.0]),
            line([14.0, 6.0], [10.0, 12.0]),
            line([10.0, 12.0], [6.0, 6.0]),
        ],
        form,
    ));
    let err = try_chain(&closed).unwrap_err();
    assert!(err.message.contains("open"), "{}", err.message);
}

/// Through the Design bench: a Rib from a line across a channel fills to
/// its walls and floor, as its op does, and a mirror copies it.
#[test]
fn a_rib_feature_fills_a_channel_from_its_sketch() {
    use super::common::{Features, body_volume, registry, settle_as_host};
    use core_document::Document;
    use wb_sketch::SketchFeature;
    use wb_sketch::sketch::{GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D};
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Bracket".into()));
    let slab = |l, w, h| PrimitiveKind::Box {
        length: l,
        width: w,
        height: h,
    };
    doc.add_in(
        body,
        "Floor",
        primitive_feature(slab(20.0, 20.0, 2.0), [0.0, -10.0, 0.0]),
    );
    doc.add_in(
        body,
        "Left",
        primitive_feature(slab(2.0, 20.0, 20.0), [0.0, -10.0, 0.0]),
    );
    doc.add_in(
        body,
        "Right",
        primitive_feature(slab(2.0, 20.0, 20.0), [18.0, -10.0, 0.0]),
    );
    let plane = SketchPlane::from_frame([0.0, -5.0, 0.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0]);
    let mut line = Sketch::new("Web");
    line.plane = plane;
    let ends = [(6.0, 12.0), (14.0, 12.0)]
        .map(|(x, y)| line.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y)))));
    line.add_geometry(GeometryElement::Line(Line::new(ends[0], ends[1])));
    let sketch = doc.add_in(body, "Web", SketchFeature::new(line, plane));
    let rib = doc.add_in(
        body,
        "Rib",
        wb_design::DesignFeature::Rib {
            sketch,
            thickness: 2.0,
            side: RibSide::Both,
            along_normal: false,
            refine: true,
            boolean_tolerance: None,
        },
    );
    doc.mark_feature_dirty(rib);
    settle_as_host(&registry, &mut doc);
    let error = &doc.get_feature_meta(rib).unwrap().error;
    assert!(error.is_none(), "it builds: {error:?}");
    let got = body_volume(&doc, body);
    let web = 16.0 * 10.0 * 2.0;
    assert!(
        (got - (CHANNEL + web)).abs() < 1e-3,
        "{got}, want {}",
        CHANNEL + web
    );

    let mirror = doc.add_in(
        body,
        "Mirrored",
        wb_design::DesignFeature::Mirrored {
            skipped: Vec::new(),
            refine: false,
            boolean_tolerance: None,
            originals: vec![rib],
            plane: wb_design::MirrorPlane::XZ,
        },
    );
    doc.mark_feature_dirty(mirror);
    settle_as_host(&registry, &mut doc);
    let got = body_volume(&doc, body);
    assert!(
        (got - (CHANNEL + 2.0 * web)).abs() < 1e-3,
        "{got}, want {}",
        CHANNEL + 2.0 * web
    );
}
