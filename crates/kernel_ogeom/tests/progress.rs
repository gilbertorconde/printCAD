//! What a running job tells the host, and how it stops.
//!
//! printCAD's op-level labels and the kernel's own stages share one channel:
//! whatever `Watch` is installed on the calling thread. These tests install a
//! collecting sink the way `app_shell`'s kernel worker does.

use std::sync::{Arc, Mutex};

use kernel_api::{
    BooleanOp, ExtrudeTermination, Kernel, Profile, ProfilePlane, ProfileSegment, ProfileWire,
    SolidOp, SweepKind, TessellationSettings,
};
use kernel_ogeom::{OgeomKernel, Watch, CONTEXT_PREFIX};

fn new_kernel() -> OgeomKernel {
    let mut kernel = OgeomKernel::new();
    kernel.initialize().expect("initialize kernel");
    kernel
}

fn xy_plane() -> ProfilePlane {
    ProfilePlane {
        origin: [0.0, 0.0, 0.0],
        x_axis: [1.0, 0.0, 0.0],
        y_axis: [0.0, 1.0, 0.0],
        normal: [0.0, 0.0, 1.0],
    }
}

fn rect_wire(x0: f64, y0: f64, x1: f64, y1: f64) -> ProfileWire {
    ProfileWire {
        segments: vec![
            ProfileSegment::Line {
                start: [x0, y0],
                end: [x1, y0],
            },
            ProfileSegment::Line {
                start: [x1, y0],
                end: [x1, y1],
            },
            ProfileSegment::Line {
                start: [x1, y1],
                end: [x0, y1],
            },
            ProfileSegment::Line {
                start: [x0, y1],
                end: [x0, y0],
            },
        ],
    }
}

fn pad(wire: ProfileWire, distance: f64, op: BooleanOp) -> SolidOp {
    SolidOp::Sweep {
        profile: Profile {
            plane: xy_plane(),
            wires: vec![wire],
        },
        kind: SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        },
        op,
    }
}

/// A pad, then an overlapping pad fused onto it — two ops, and the fuse makes
/// the kernel announce its own boolean stages too.
fn two_op_chain() -> Vec<SolidOp> {
    vec![
        pad(rect_wire(0.0, 0.0, 10.0, 10.0), 5.0, BooleanOp::NewSolid),
        pad(rect_wire(5.0, 5.0, 15.0, 15.0), 5.0, BooleanOp::Fuse),
    ]
}

/// Collect everything announced while `job` runs.
fn heard_while<T>(job: impl FnOnce() -> T) -> (T, Vec<String>) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let collector = Arc::clone(&heard);
    let watch = Watch::with_sink(move |stage: &str| {
        collector.lock().expect("sink lock").push(stage.to_owned());
    });
    let out = kernel_ogeom::watched(&watch, job);
    let heard = heard.lock().expect("sink lock").clone();
    (out, heard)
}

#[test]
fn a_rebuild_announces_each_feature_in_order() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let ops = two_op_chain();

    let (result, heard) = heard_while(|| kernel.execute_solid_chain(&ops, &detail));
    result.expect("two-op chain builds");

    let ours: Vec<String> = heard
        .iter()
        .filter_map(|s| s.strip_prefix(CONTEXT_PREFIX).map(str::to_owned))
        .collect();
    assert!(
        ours.starts_with(&["Pad 1/2".to_string(), "Pad 2/2".to_string()]),
        "each op announces its feature name and position, in order: {ours:?}"
    );
    assert!(
        ours.iter().any(|s| s.starts_with("Meshing ")),
        "the final mesh is announced too: {ours:?}"
    );
}

#[test]
fn the_kernels_own_stages_arrive_alongside_ours() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let ops = two_op_chain();

    let (result, heard) = heard_while(|| kernel.execute_solid_chain(&ops, &detail));
    result.expect("two-op chain builds");

    let kernel_stages: Vec<&String> = heard
        .iter()
        .filter(|s| !s.starts_with(CONTEXT_PREFIX))
        .collect();
    assert!(
        kernel_stages.iter().any(|s| s.starts_with("boolean:")),
        "the fuse should report its own stages: {heard:?}"
    );
}

#[test]
fn nothing_is_announced_when_no_watch_is_installed() {
    // The library must stay silent for callers that never install a watch —
    // which is every other test in this crate.
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    kernel
        .execute_solid_chain(&two_op_chain(), &detail)
        .expect("chain builds unwatched");
}

#[test]
fn a_cancelled_job_stops_and_says_so() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let ops = two_op_chain();

    let watch = Watch::new();
    let canceller = watch.canceller();
    // Cancel up front: the chain stops at its first checkpoint rather than
    // running to completion.
    canceller.cancel();

    let result = kernel_ogeom::watched(&watch, || kernel.execute_solid_chain(&ops, &detail));

    let err = result.expect_err("a cancelled chain must not report success");
    assert!(
        err.message.contains("cancelled"),
        "the host distinguishes a cancellation from a geometry failure by this \
         message: {err:?}"
    );
}

#[test]
fn cancelling_from_another_thread_stops_the_job() {
    let mut kernel = new_kernel();
    let detail = TessellationSettings::default();
    let ops = two_op_chain();

    let watch = Watch::new();
    let canceller = watch.canceller();
    let stopper = std::thread::spawn(move || canceller.cancel());
    stopper.join().expect("stopper thread");

    let result = kernel_ogeom::watched(&watch, || kernel.execute_solid_chain(&ops, &detail));
    assert!(
        result.is_err_and(|e| e.message.contains("cancelled")),
        "a canceller sent to another thread still stops the job"
    );
}
