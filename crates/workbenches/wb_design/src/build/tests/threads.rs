//! The plan tests of threads on a face.

use super::*;
use kernel_api::FaceThread;

/// A thread on the side of a pad, cut to `thread` (none for its own
/// pitch and depth).
fn thread_on(face_x: f32, thread: Option<ThreadSpec>) -> DesignFeature {
    DesignFeature::Thread {
        face: FacePick {
            point: [face_x, 2.5, 3.0],
            normal: [1.0, 0.0, 0.0],
            name: 0,
        },
        thread,
        pitch: 0.8,
        depth: 0.4,
        length: 4.0,
        full_length: true,
        from_far_end: false,
        left_handed: false,
        chamfer: false,
        run_out: false,
        cosmetic: false,
        refine: false,
        boolean_tolerance: None,
    }
}

fn edited(mut feature: DesignFeature, edit: impl FnOnce(&mut DesignFeature)) -> DesignFeature {
    edit(&mut feature);
    feature
}

/// The thread op the body's plan ends with.
fn thread_op(doc: &Document, body: BodyId) -> (FaceProbe, FaceThread) {
    let plan = body_build_ops(doc, body).unwrap();
    match plan.ops.last() {
        Some(SolidOp::Thread { face, form }) => (*face, *form),
        other => panic!("a thread op, not {other:?}"),
    }
}

/// A standard size gives the thread its pitch and its depth from major to
/// minor diameter; with none it cuts its own. A length, the far end, the
/// hand, a chamfer and a run-out reach the op as they are set.
#[test]
fn a_thread_cuts_its_standard_size_or_its_own_pitch() {
    let (mut doc, body, sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch, 6.0), "Pad");
    let m6 = ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6");
    let id = add(&mut doc, body, thread_on(10.0, Some(m6.clone())), "Thread");
    let (face, form) = thread_op(&doc, body);
    assert_eq!(face.point, [10.0, 2.5, 3.0]);
    assert_eq!(form.pitch, 1.0);
    assert!(
        (form.depth - 1.082_532 * 0.5).abs() < 1e-6,
        "{}",
        form.depth
    );
    assert_eq!(form.flank_deg, 60.0);
    assert_eq!(
        (form.length, form.left_handed, form.chamfer, form.run_out),
        (None, false, false, 0.0)
    );

    let own = edited(thread_on(10.0, None), |f| {
        if let DesignFeature::Thread {
            full_length,
            from_far_end,
            left_handed,
            chamfer,
            run_out,
            ..
        } = f
        {
            (*full_length, *from_far_end, *left_handed) = (false, true, true);
            (*chamfer, *run_out) = (true, true);
        }
    });
    doc.update_feature_data(id, own.to_json()).unwrap();
    let (_, form) = thread_op(&doc, body);
    assert_eq!((form.pitch, form.depth), (0.8f32.into(), 0.4f32.into()));
    assert_eq!(form.length, Some(4.0));
    assert!(form.from_far_end && form.left_handed && form.chamfer);
    assert!(
        (form.run_out - 1.6).abs() < 1e-6,
        "two pitches: {}",
        form.run_out
    );

    // A standard size's own left hand counts.
    let mut left = m6;
    left.left_handed = true;
    doc.update_feature_data(id, thread_on(10.0, Some(left)).to_json())
        .unwrap();
    assert!(thread_op(&doc, body).1.left_handed);
}

/// A cosmetic thread cuts nothing and is called out by its size; a taper
/// standard is a Hole's, and refused on the thread.
#[test]
fn a_cosmetic_thread_cuts_nothing_and_a_taper_one_is_refused() {
    let (mut doc, body, sketch) = doc_with_body_sketch();
    add(&mut doc, body, pad(sketch, 6.0), "Pad");
    let m6 = ThreadSpec::new(ThreadStandard::IsoMetricCoarse, "M6");
    let cosmetic = edited(thread_on(10.0, Some(m6)), |f| {
        if let DesignFeature::Thread {
            cosmetic,
            left_handed,
            ..
        } = f
        {
            (*cosmetic, *left_handed) = (true, true);
        }
    });
    assert_eq!(cosmetic.cosmetic_thread().as_deref(), Some("M6 LH"));
    let id = add(&mut doc, body, cosmetic, "Thread");
    let plan = body_build_ops(&doc, body).unwrap();
    assert!(
        !plan
            .ops
            .iter()
            .any(|op| matches!(op, SolidOp::Thread { .. })),
        "{:?}",
        plan.ops
    );
    let unc = ThreadSpec::new(ThreadStandard::Unc, "1/4-20");
    assert_eq!(
        crate::build::thread_designation(&thread_on(1.0, Some(unc))).as_deref(),
        Some("1/4-20 UNC")
    );

    let npt = ThreadSpec::new(ThreadStandard::Npt, "1/4");
    doc.update_feature_data(id, thread_on(10.0, Some(npt)).to_json())
        .unwrap();
    let err = body_build_ops(&doc, body).unwrap_err();
    assert_eq!(err.feature, Some(id));
    assert!(err.message.contains("taper"), "{}", err.message);
}
