//! Stage labels announced while a job runs.
//!
//! The kernel's `progress::stage` writes to whatever `Watch` is installed on
//! the current thread, so printCAD's own op-level labels ride the same
//! channel as ogeom's internal stages (`boolean: intersect`, `step: solid`,
//! …). The host installs one watch per job and hears both.
//!
//! Our labels carry [`CONTEXT_PREFIX`] so a sink can tell the two apart: ours
//! say *which feature* is building and are worth keeping on screen, the
//! kernel's say *what it is doing right now* and change rapidly.
//!
//! Every call here is a no-op when no watch is installed, which is why tests
//! and library callers need not care.
//!
//! The kernel's sink carries only a name — no counts, no fraction — so any
//! "3/7" in a label is counted on this side (ogeom-rs#9).

use std::fmt::Display;

use kernel_api::{BooleanOp, SolidOp, SweepKind};

/// Marks a label as printCAD's own rather than one of the kernel's stages.
pub const CONTEXT_PREFIX: &str = "printcad: ";

/// Announce which piece of work is starting.
pub fn context(label: impl Display) {
    ogeom::core::progress::stage(&format!("{CONTEXT_PREFIX}{label}"));
}

/// Offer the host a chance to stop, between items of one of our own loops.
///
/// The kernel checkpoints inside booleans, marching and its own import loops,
/// but not inside `triangulate_face` — and our op, solid and face loops are
/// the outer ones anyway, so they are the right place to ask.
///
/// Returns the kernel's `cancelled` message when the watch has been cancelled;
/// free when unwatched.
/// Announce a counted stage: `(done, total)` under a stable name, for a
/// determinate progress bar. Safe from parallel workers as long as `done`
/// comes from a shared monotone counter.
pub fn stage_at(name: &str, done: u64, total: u64) {
    ogeom::core::progress::stage_at(name, done, total);
}

pub fn checkpoint() -> Result<(), String> {
    ogeom::core::progress::checkpoint().map_err(|e| e.to_string())
}

/// What to call an op in the status line — the feature name the user knows it
/// by, which for sweeps depends on whether it adds or removes material.
pub fn op_label(op: &SolidOp) -> &'static str {
    let subtractive = op.boolean_op() == Some(BooleanOp::Cut);
    match op {
        SolidOp::Sweep { kind, .. } => match kind {
            SweepKind::Extrude { .. } if subtractive => "Pocket",
            SweepKind::Extrude { .. } => "Pad",
            SweepKind::Revolve { .. } if subtractive => "Groove",
            SweepKind::Revolve { .. } => "Revolution",
            SweepKind::Helix { .. } => "Helix",
        },
        SolidOp::Loft { .. } => "Loft",
        SolidOp::Pipe { .. } => "Pipe",
        SolidOp::Primitive { .. } => "Primitive",
        SolidOp::Fillet { .. } => "Fillet",
        SolidOp::Chamfer { .. } => "Chamfer",
        SolidOp::Draft { .. } => "Draft",
        SolidOp::Thickness { .. } => "Thickness",
        SolidOp::Transform { .. } => "Pattern",
        SolidOp::Boolean { .. } => "Boolean",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel_api::{ExtrudeTermination, Profile, ProfilePlane};

    fn sweep(kind: SweepKind, op: BooleanOp) -> SolidOp {
        SolidOp::Sweep {
            profile: Profile {
                plane: ProfilePlane {
                    origin: [0.0; 3],
                    x_axis: [1.0, 0.0, 0.0],
                    y_axis: [0.0, 1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                wires: Vec::new(),
            },
            kind,
            op,
        }
    }

    fn extrude() -> SweepKind {
        SweepKind::Extrude {
            termination: ExtrudeTermination::Blind { distance: 1.0 },
            second_side: None,
            symmetric: false,
            reversed: false,
            taper_deg: 0.0,
            direction: None,
        }
    }

    fn revolve() -> SweepKind {
        SweepKind::Revolve {
            axis_origin: [0.0, 0.0],
            axis_dir: [0.0, 1.0],
            angle_deg: 360.0,
            second_angle_deg: None,
            midplane: false,
            reversed: false,
        }
    }

    #[test]
    fn a_sweeps_label_follows_whether_it_adds_or_removes_material() {
        assert_eq!(op_label(&sweep(extrude(), BooleanOp::NewSolid)), "Pad");
        assert_eq!(op_label(&sweep(extrude(), BooleanOp::Fuse)), "Pad");
        assert_eq!(op_label(&sweep(extrude(), BooleanOp::Cut)), "Pocket");
        assert_eq!(op_label(&sweep(revolve(), BooleanOp::Fuse)), "Revolution");
        assert_eq!(op_label(&sweep(revolve(), BooleanOp::Cut)), "Groove");
    }

    #[test]
    fn a_counted_stage_reaches_a_stage_sink_with_its_counts() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let collector = std::sync::Arc::clone(&heard);
        let watch = ogeom::core::progress::Watch::with_stage_sink(
            move |stage: ogeom::core::progress::Stage<'_>| {
                collector
                    .lock()
                    .expect("sink lock")
                    .push((stage.name.to_owned(), stage.progress));
            },
        );
        ogeom::core::progress::watched(&watch, || {
            context("Preparing 3 bodies");
            stage_at("bodies", 1, 3);
        });

        let heard = heard.lock().expect("sink lock");
        assert_eq!(heard.len(), 2);
        assert!(heard[0].0.starts_with(CONTEXT_PREFIX));
        assert_eq!(heard[0].1, None, "a context is a bare boundary");
        assert_eq!(heard[1], ("bodies".to_owned(), Some((1, 3))));
    }

    #[test]
    fn a_context_label_is_prefixed_so_a_sink_can_tell_it_from_a_kernel_stage() {
        let heard = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let collector = std::sync::Arc::clone(&heard);
        let watch = ogeom::core::progress::Watch::with_sink(move |s: &str| {
            collector.lock().expect("sink lock").push(s.to_owned());
        });
        ogeom::core::progress::watched(&watch, || context("Pad 1/3"));

        let heard = heard.lock().expect("sink lock");
        assert_eq!(heard.len(), 1);
        assert_eq!(
            heard[0].strip_prefix(CONTEXT_PREFIX),
            Some("Pad 1/3"),
            "our labels must be distinguishable from the kernel's stages"
        );
    }

    #[test]
    fn announcing_without_a_watch_is_a_no_op() {
        context("Pad 1/1");
    }
}
