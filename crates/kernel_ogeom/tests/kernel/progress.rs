//! What a running job tells the host, and how it stops.
//!
//! printCAD's op-level labels and the kernel's own stages share one channel:
//! whatever `Watch` is installed on the calling thread. These tests install a
//! collecting sink the way `app_shell`'s kernel worker does.

use std::sync::{Arc, Mutex};

use kernel_api::{
    BooleanOp, ExtrudeTermination, Profile, ProfileWire, SolidOp, SweepKind, TessellationSettings,
};
use kernel_ogeom::{CONTEXT_PREFIX, Watch};

use super::common::{new_kernel, rect_wire, xy_plane};

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

/// A pad, then an overlapping pad fused onto it: two ops, and the fuse makes
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
    // The mesh pass is a detail beneath the last op, not a context of its
    // own: unprefixed, so a sink shows it while the pass runs and drops it
    // the moment a counted stage takes the display.
    assert!(
        heard.iter().any(|s| s.starts_with("Meshing ")),
        "the final mesh is announced as a detail: {heard:?}"
    );
    // The kernel's own stages arrive alongside ours, unprefixed.
    assert!(
        heard
            .iter()
            .any(|s| !s.starts_with(CONTEXT_PREFIX) && s.starts_with("boolean:")),
        "the fuse should report its own stages: {heard:?}"
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

    // Cancelled before it runs, the chain stops at its first checkpoint
    // rather than running to completion, and says so: the host tells a
    // cancellation from a geometry failure by this message.
    let result = kernel_ogeom::watched(&watch, || kernel.execute_solid_chain(&ops, &detail));
    assert!(
        result.is_err_and(|e| e.message.contains("cancelled")),
        "a canceller sent to another thread still stops the job"
    );
}
