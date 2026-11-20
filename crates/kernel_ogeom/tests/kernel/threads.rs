//! Threads cut into a cylindrical face, measured against the volume
//! their groove's section takes turned round the face's axis.

use std::f64::consts::{PI, TAU};

use kernel_api::{BooleanOp, FaceProbe, FaceThread, Placement, PrimitiveKind, SolidOp};

use super::common::{chain, is_sound, primitive, primitive_feature, try_chain, volume};

/// A shaft `radius` across and `height` tall on the XY plane, its axis Z.
fn shaft(radius: f64, height: f64) -> SolidOp {
    primitive(PrimitiveKind::Cylinder {
        radius,
        height,
        angle_deg: 360.0,
    })
}

/// A point of the side of a shaft or bore of `radius` about Z at height
/// `z`, facing out of the material.
fn side_at(radius: f64, z: f64, external: bool) -> FaceProbe {
    FaceProbe {
        point: [radius, 0.0, z],
        normal: if external {
            [1.0, 0.0, 0.0]
        } else {
            [-1.0, 0.0, 0.0]
        },
        name: 0,
    }
}

/// An ISO metric thread of `pitch`: the groove 5/8 of the fundamental
/// triangle deep, 60° between its flanks.
fn metric(pitch: f64) -> FaceThread {
    FaceThread {
        pitch,
        depth: 1.082_532 * pitch * 0.5,
        flank_deg: 60.0,
        ..Default::default()
    }
}

/// What a thread's groove takes out of the material over `length` of its
/// face: each axial section of it, an annulus between the face (`radius`)
/// and the groove's root, takes the groove's share of that annulus turned
/// once a pitch, so the volume is the groove's section in the material
/// turned round the axis, once per pitch of length.
fn groove_volume(radius: f64, form: &FaceThread, external: bool, length: f64) -> f64 {
    let (p, h) = (form.pitch, form.depth);
    let tan = (form.flank_deg * 0.5).to_radians().tan();
    let crest = p / 16.0;
    let root = if external { radius - h } else { radius + h };
    // ∫ 2πρ · 2(crest + |ρ − root| tan) dρ between the face and the root.
    let steps = 2000;
    let (a, b) = if external {
        (root, radius)
    } else {
        (radius, root)
    };
    let dr = (b - a) / steps as f64;
    let turned: f64 = (0..steps)
        .map(|i| {
            let rho = a + (i as f64 + 0.5) * dr;
            TAU * rho * 2.0 * (crest + (rho - root).abs() * tan) * dr
        })
        .sum();
    turned * length / p
}

fn thread(face: FaceProbe, form: FaceThread) -> SolidOp {
    SolidOp::Thread { face, form }
}

/// An M6 thread down the whole of a 3 mm shaft takes the groove's share
/// of the ring between the major and minor diameters, its length's worth.
#[test]
fn an_external_thread_takes_its_groove_out_of_the_shaft() {
    let (r, h) = (3.0, 4.0);
    let form = metric(1.0);
    let built = chain(&[shaft(r, h), thread(side_at(r, 1.0, true), form)]);
    assert!(is_sound(&built.brep_blob), "a closed solid");
    let removed = PI * r * r * h - volume(&built.brep_blob);
    let want = groove_volume(r, &form, true, h);
    assert!(
        (removed - want).abs() < want * 0.01,
        "removed {removed}, the groove's {want}"
    );
    let ring = PI * (r * r - (r - form.depth).powi(2)) * h;
    assert!(
        removed < ring * 0.7 && removed > ring * 0.3,
        "{removed} of {ring}"
    );
}

/// A bore through a block threaded its whole length takes the internal
/// groove's share of the ring out past the bore.
#[test]
fn an_internal_thread_cuts_out_into_a_bore() {
    let (r, h) = (2.5, 4.0);
    let block = SolidOp::Primitive {
        kind: PrimitiveKind::Box {
            length: 12.0,
            width: 12.0,
            height: h,
        },
        placement: Placement {
            origin: [-6.0, -6.0, 0.0],
            ..Placement::default()
        },
        op: BooleanOp::NewSolid,
    };
    let bore = SolidOp::Primitive {
        kind: PrimitiveKind::Cylinder {
            radius: r,
            height: h,
            angle_deg: 360.0,
        },
        placement: Placement::default(),
        op: BooleanOp::Cut,
    };
    let form = metric(1.0);
    let drilled = volume(&chain(&[block.clone(), bore.clone()]).brep_blob);
    let built = chain(&[block, bore, thread(side_at(r, 1.0, false), form)]);
    assert!(is_sound(&built.brep_blob), "a closed solid");
    let removed = drilled - volume(&built.brep_blob);
    let want = groove_volume(r, &form, false, h);
    assert!(
        (removed - want).abs() < want * 0.01,
        "removed {removed}, the groove's {want}"
    );
}

/// A thread wants a cylindrical face: on a box's face it fails, saying
/// so.
#[test]
fn a_thread_on_a_flat_face_fails_on_its_op() {
    let block = primitive(PrimitiveKind::Box {
        length: 10.0,
        width: 10.0,
        height: 10.0,
    });
    let face = FaceProbe {
        point: [10.0, 5.0, 5.0],
        normal: [1.0, 0.0, 0.0],
        name: 0,
    };
    let err = try_chain(&[block, thread(face, metric(1.0))]).unwrap_err();
    assert_eq!(err.op_index, 1);
    assert!(err.message.contains("cylindrical"), "{}", err.message);
}

/// A Thread feature on the side of a shaft of `radius` about Z, picked at
/// height 1, cut to `size` of ISO metric coarse.
fn thread_feature(radius: f32, size: &str, cosmetic: bool) -> wb_design::DesignFeature {
    wb_design::DesignFeature::Thread {
        face: wb_design::FacePick {
            point: [radius, 0.0, 1.0],
            normal: [1.0, 0.0, 0.0],
            name: 0,
        },
        thread: Some(wb_design::ThreadSpec::new(
            wb_design::ThreadStandard::IsoMetricCoarse,
            size,
        )),
        pitch: 1.0,
        depth: 0.5,
        length: 10.0,
        full_length: true,
        from_far_end: false,
        left_handed: false,
        chamfer: false,
        run_out: false,
        cosmetic,
        refine: false,
        boolean_tolerance: None,
    }
}

/// A cosmetic thread repeated with its shaft by a pattern is called out
/// on each copy, as a cosmetic hole's copies are; a copy the pattern
/// skips is not.
#[test]
fn a_patterned_cosmetic_thread_is_called_out_on_every_copy() {
    use super::common::{Features, registry, settle_as_host};
    use core_document::Document;
    let (r, h) = (3.0, 4.0);
    let registry = registry();
    let mut doc = Document::new("t");
    let body = doc.create_body(Some("Bolts".into()));
    let shaft = doc.add_in(
        body,
        "Shaft",
        primitive_feature(
            PrimitiveKind::Cylinder {
                radius: r,
                height: h,
                angle_deg: 360.0,
            },
            [0.0; 3],
        ),
    );
    let thread = doc.add_in(body, "Thread", thread_feature(r as f32, "M6", true));
    let pattern = |skipped: Vec<u32>| wb_design::DesignFeature::LinearPattern {
        originals: vec![shaft, thread],
        axis: wb_design::PatternAxis::X,
        length: 20.0,
        occurrences: 3,
        spacing_mode: false,
        reversed: false,
        refine: false,
        boolean_tolerance: None,
        spacings: Vec::new(),
        skipped,
    };
    let copies = doc.add_in(body, "Pattern", pattern(Vec::new()));
    settle_as_host(&registry, &mut doc);
    let mut mouths: Vec<f64> = doc
        .derived()
        .cosmetic_threads()
        .iter()
        .map(|c| {
            assert_eq!(c.designation, "M6");
            assert!((c.radius - r).abs() < 1e-3, "{}", c.radius);
            c.mouth[0]
        })
        .collect();
    mouths.sort_by(f64::total_cmp);
    assert_eq!(mouths.len(), 3, "{mouths:?}");
    assert!((mouths[1] - 10.0).abs() < 1e-3 && (mouths[2] - 20.0).abs() < 1e-3);

    doc.set_feature(copies, pattern(vec![2]));
    doc.mark_feature_dirty(copies);
    settle_as_host(&registry, &mut doc);
    assert_eq!(
        doc.derived().cosmetic_threads().len(),
        2,
        "the skipped copy has none"
    );
}

/// Threads cut along their whole length (many turns, long builds): run
/// with the package's `heavy-tests` feature (`node scripts/test-budget.mjs --heavy`).
#[cfg(feature = "heavy-tests")]
mod heavy {
    use super::*;

    /// A thread given a length stops there, from the end of the face nearer
    /// where it was picked, or from the other end when told.
    #[test]
    fn a_thread_runs_its_length_from_the_end_it_starts_at() {
        let (r, h) = (3.0, 6.0);
        let form = FaceThread {
            length: Some(2.0),
            ..metric(1.0)
        };
        let want = groove_volume(r, &form, true, 2.0);
        for (z, far, bottom) in [(1.0, false, true), (1.0, true, false), (5.0, false, false)] {
            let built = chain(&[
                shaft(r, h),
                thread(
                    side_at(r, z, true),
                    FaceThread {
                        from_far_end: far,
                        ..form
                    },
                ),
            ]);
            let removed = PI * r * r * h - volume(&built.brep_blob);
            assert!(
                (removed - want).abs() < want * 0.01,
                "picked at {z}, far {far}: removed {removed}, want {want}"
            );
            // The groove's root shows in the mesh within a millimetre of the
            // end the thread starts at, and not near the other.
            let root = (r - form.depth) as f32;
            let grooved_near = |z0: f32| {
                built.mesh.positions.iter().any(|p| {
                    let rho = p[0].hypot(p[1]);
                    (p[2] - z0).abs() < 1.0 && rho > root - 0.01 && rho < root + 0.2
                })
            };
            let (low_end, high_end) = (grooved_near(0.0), grooved_near(h as f32));
            assert!(
                low_end == bottom && high_end == !bottom,
                "picked at {z}, far {far}: the thread starts at the {} end",
                if bottom { "lower" } else { "upper" }
            );
        }
    }

    /// A right-hand thread climbs anticlockwise about its axis seen from the
    /// end it climbs toward, whichever end it starts from, and a left-hand
    /// one clockwise: the groove's root, at height z and angle θ, keeps
    /// 2πz/P − θ for a right hand (2πz/P + θ for a left) the same all along.
    #[test]
    fn a_thread_turns_the_hand_it_is_given() {
        let (r, h, p) = (3.0, 3.0, 1.0);
        for (far, left) in [(false, false), (true, false), (false, true)] {
            let form = FaceThread {
                from_far_end: far,
                left_handed: left,
                ..metric(p)
            };
            let built = chain(&[shaft(r, h), thread(side_at(r, 1.0, true), form)]);
            let root = r - form.depth;
            // How closely the root's points keep one phase, for each hand: one
            // when they all keep it, near none when they spread round.
            let kept = |sign: f64| {
                let (mut c, mut s, mut n) = (0.0, 0.0, 0.0);
                for q in &built.mesh.positions {
                    let q = q.map(f64::from);
                    if (q[0].hypot(q[1]) - root).abs() > 0.01 || q[2] < 0.2 || q[2] > h - 0.2 {
                        continue;
                    }
                    let phase = TAU * q[2] / p - sign * q[1].atan2(q[0]);
                    (c, s, n) = (c + phase.cos(), s + phase.sin(), n + 1.0);
                }
                assert!(n > 10.0, "the root shows");
                c.hypot(s) / n
            };
            let (right, left_kept) = (kept(1.0), kept(-1.0));
            let (wanted, other) = if left {
                (left_kept, right)
            } else {
                (right, left_kept)
            };
            assert!(
                wanted > 0.9 && other < 0.5,
                "far {far}, left {left}: right-hand phase kept {right}, left-hand {left_kept}"
            );
        }
    }

    /// A chamfer takes the starting end's corner off down to the root, and a
    /// run-out leaves the groove's last two pitches shallower: each takes
    /// some material and not the whole ring.
    #[test]
    fn a_chamfer_and_a_run_out_change_the_ends() {
        let (r, h) = (3.0, 4.0);
        let plain = metric(1.0);
        let cut = |form: FaceThread| {
            let built = chain(&[shaft(r, h), thread(side_at(r, 1.0, true), form)]);
            PI * r * r * h - volume(&built.brep_blob)
        };
        let base = cut(plain);
        let chamfered = cut(FaceThread {
            chamfer: true,
            ..plain
        });
        // The 45° cone takes what the groove left of the end's corner.
        let corner = PI * plain.depth * plain.depth * (r - plain.depth / 3.0);
        assert!(
            chamfered > base && chamfered < base + corner,
            "chamfered {chamfered}, plain {base}, the corner {corner}"
        );
        let run_out = cut(FaceThread {
            run_out: 2.0,
            ..plain
        });
        assert!(
            run_out < base && run_out > base * 0.6,
            "run out {run_out}, plain {base}"
        );
    }

    /// Through the Design bench as the app rebuilds it: an M6 Thread on a
    /// 6 mm shaft cuts the M6 groove; made cosmetic, it cuts nothing and the
    /// document calls out an M6 thread at the shaft's end.
    #[test]
    fn a_thread_feature_cuts_its_size_or_is_called_out() {
        use super::super::common::{Features, body_volume, registry, settle_as_host};
        use core_document::{Document, WorkbenchFeature};
        let (r, h) = (3.0, 4.0);
        let registry = registry();
        let mut doc = Document::new("t");
        let body = doc.create_body(Some("Bolt".into()));
        doc.add_in(
            body,
            "Shaft",
            primitive_feature(
                PrimitiveKind::Cylinder {
                    radius: r,
                    height: h,
                    angle_deg: 360.0,
                },
                [0.0; 3],
            ),
        );
        let thread = doc.add_in(body, "Thread", thread_feature(r as f32, "M6", false));
        settle_as_host(&registry, &mut doc);
        let error = doc.derived().error(thread);
        assert!(error.is_none(), "it builds: {error:?}");
        let removed = PI * r * r * h - body_volume(&doc, body);
        let want = groove_volume(r, &metric(1.0), true, h);
        assert!(
            (removed - want).abs() < want * 0.01,
            "removed {removed}, want {want}"
        );
        assert!(doc.derived().cosmetic_threads().is_empty());

        doc.update_feature_data(thread, thread_feature(r as f32, "M6", true).to_json())
            .unwrap();
        doc.mark_feature_dirty(thread);
        settle_as_host(&registry, &mut doc);
        assert!((body_volume(&doc, body) - PI * r * r * h).abs() < 1e-6);
        let callouts = doc.derived().cosmetic_threads();
        assert_eq!(callouts.len(), 1, "{callouts:?}");
        let callout = &callouts[0];
        assert_eq!(callout.designation, "M6");
        assert!((callout.radius - r).abs() < 1e-3, "{}", callout.radius);
        assert!(
            callout.mouth.iter().all(|c| c.abs() < 1e-3) && (callout.axis[2] - 1.0).abs() < 1e-6,
            "at the end it was picked nearer, into the shaft: {callout:?}"
        );
    }
}
