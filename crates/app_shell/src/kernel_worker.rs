//! Background worker that owns the geometry kernel.
//!
//! Kernel work is fully CPU-bound (STEP parsing + tessellation). Running it
//! on the UI thread freezes the viewport for tens of seconds on big models;
//! moving it onto a dedicated worker keeps panning/orbiting smooth during
//! imports.
//!
//! The worker also performs the `std::fs::read(path)` that backs the
//! document's asset blob — that I/O belongs off the UI thread, and is
//! naturally cheap to colocate with the kernel call since the worker is
//! already off the hot path.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use std::sync::{Arc, Mutex};

use kernel_api::{ImportedModel, Kernel, SolidBuildResult, SolidOp, TessellationSettings};
use kernel_ogeom::{Canceller, OgeomKernel, Watch};
use tracing::info;
use uuid::Uuid;

/// Job submitted from the UI thread to the kernel worker.
pub enum KernelRequest {
    /// Read a STEP/STP file (BRep-only fast path) and return bodies for the UI
    /// to register; tessellation is scheduled separately per body.
    ImportStep {
        path: PathBuf,
        detail: TessellationSettings,
    },
    /// Rebuild a body's solid from its Part Design feature chain.
    /// `op_features` maps each op index to its owning feature id so a
    /// failure can be pinned on the culprit in the tree.
    BuildSolid {
        body_id: Uuid,
        ops: Vec<SolidOp>,
        op_features: Vec<Uuid>,
        detail: TessellationSettings,
        /// The feature being edited, whose preview the result carries.
        preview: Option<Uuid>,
    },
    /// Measure a body's snapshot: volume, area, centre of mass.
    Measure {
        body_id: Uuid,
        revision: u64,
        brep_blob: Arc<Vec<u8>>,
    },
    /// Build a B-rep solid from a mesh body's triangles.
    MeshToSolid {
        body_id: Uuid,
        mesh: Arc<kernel_api::TriMesh>,
        detail: TessellationSettings,
    },
    /// Run the kernel's repair on an imported body's snapshot.
    RepairShape {
        body_id: Uuid,
        brep_blob: Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
        detail: TessellationSettings,
    },
}

/// Result delivered from the worker back to the UI thread.
///
/// Each request emits exactly one response; the UI's `in_flight` counter is
/// decremented when one is drained, so spurious extras would skew the
/// "importing X..." indicator.
pub enum KernelResponse {
    StepImported {
        path: PathBuf,
        model: ImportedModel,
        raw_bytes: Vec<u8>,
        detail: TessellationSettings,
        elapsed: Duration,
    },
    StepFailed {
        path: PathBuf,
        error: String,
    },
    SolidBuilt {
        body_id: Uuid,
        result: SolidBuildResult,
        elapsed: Duration,
    },
    SolidFailed {
        body_id: Uuid,
        failed_feature: Option<Uuid>,
        error: String,
    },
    ShapeRepaired {
        body_id: Uuid,
        result: kernel_api::RepairResult,
        elapsed: Duration,
    },
    RepairFailed {
        body_id: Uuid,
        error: String,
    },
    Measured {
        body_id: Uuid,
        revision: u64,
        result: Result<kernel_api::PhysicalProperties, String>,
    },
    MeshSolidBuilt {
        body_id: Uuid,
        result: kernel_api::MeshSolidResult,
        elapsed: Duration,
    },
    MeshSolidFailed {
        body_id: Uuid,
        error: String,
    },
}

/// What the kernel thread is doing right now, shared with the UI thread.
///
/// The kernel announces stages through a thread-local watch (see
/// `kernel_ogeom::progress`); the sink lands them here and the UI reads the
/// composed line each frame. A shared slot rather than a channel on purpose:
/// `std::sync::mpsc::Sender` is `Send` but not `Sync`, so it cannot be
/// captured by the sink, and this keeps `in_flight`'s one-response-per-request
/// accounting untouched.
#[derive(Default)]
struct Activity {
    /// printCAD's own label: which feature or body is being worked on.
    context: Option<String>,
    /// The kernel's stage within that work — changes rapidly, and during a
    /// parallel loop arrives from every worker thread at once.
    detail: Option<String>,
    /// `(done, total)` announced by the kernel's current stage.
    progress: Option<(u64, u64)>,
    /// `(done, total)` announced by OUR counted stage (the per-body import
    /// loop). While set, it owns the display: the kernel's per-body stages
    /// beneath it are twenty threads' worth of noise, not information.
    own_progress: Option<(u64, u64)>,
    /// Stops the job currently running, when there is one.
    canceller: Option<Canceller>,
}

impl Activity {
    /// The one line worth showing: our label, refined by the kernel's stage.
    fn status(&self) -> Option<String> {
        // Our counted stage speaks alone; the bar carries its numbers.
        if self.own_progress.is_some() {
            return self.context.clone();
        }
        match (self.context.as_deref(), self.detail.as_deref()) {
            (Some(context), Some(detail)) => Some(format!("{context} · {detail}")),
            (Some(only), None) | (None, Some(only)) => Some(only.to_owned()),
            (None, None) => None,
        }
    }

    /// The counts to draw: ours when we are counting, else the kernel's.
    fn progress(&self) -> Option<(u64, u64)> {
        self.own_progress.or(self.progress)
    }
}

fn lock(activity: &Mutex<Activity>) -> std::sync::MutexGuard<'_, Activity> {
    activity
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// UI-side handle to the worker thread. `in_flight` is incremented by
/// [`Self::request_step_import`] / [`Self::request_tessellate_body`] and
/// decremented by [`Self::drain`] so the status panel can show a spinner while
/// imports are pending.
pub struct KernelWorker {
    tx: Sender<KernelRequest>,
    rx: Receiver<KernelResponse>,
    in_flight: u32,
    activity: Arc<Mutex<Activity>>,
}

impl KernelWorker {
    /// Spawn the worker thread. The thread owns its own [`OgeomKernel`] for
    /// the lifetime of the app; the channels disconnect when the UI side is
    /// dropped, which lets the worker exit cleanly.
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = channel::<KernelRequest>();
        let (resp_tx, resp_rx) = channel::<KernelResponse>();

        let activity = Arc::new(Mutex::new(Activity::default()));
        let worker_activity = Arc::clone(&activity);

        thread::Builder::new()
            .name("printcad-kernel-worker".to_string())
            .spawn(move || worker_loop(req_rx, resp_tx, worker_activity))
            .expect("failed to spawn kernel worker thread");

        Self {
            tx: req_tx,
            rx: resp_rx,
            in_flight: 0,
            activity,
        }
    }

    /// Submit a STEP import job. Returns immediately; the result will arrive
    /// via [`Self::drain`] some time later.
    pub fn request_step_import(&mut self, path: PathBuf, detail: TessellationSettings) {
        if self
            .tx
            .send(KernelRequest::ImportStep { path, detail })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Submit a Part Design solid rebuild. One response arrives per request.
    /// Submit a body's chain. With `preview`, a feature being edited, the
    /// result also carries what that feature does (its tool, and the body
    /// without it).
    pub fn request_build_solid(
        &mut self,
        body_id: Uuid,
        ops: Vec<SolidOp>,
        op_features: Vec<Uuid>,
        detail: TessellationSettings,
        preview: Option<Uuid>,
    ) {
        if self
            .tx
            .send(KernelRequest::BuildSolid {
                body_id,
                ops,
                op_features,
                detail,
                preview,
            })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Submit a repair of an imported body's shape. One response arrives
    /// per request.
    pub fn request_repair(
        &mut self,
        body_id: Uuid,
        brep_blob: Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
        detail: TessellationSettings,
    ) {
        if self
            .tx
            .send(KernelRequest::RepairShape {
                body_id,
                brep_blob,
                face_colors,
                detail,
            })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Submit the conversion of a mesh body to a solid. One response
    /// arrives per request.
    pub fn request_mesh_solid(
        &mut self,
        body_id: Uuid,
        mesh: Arc<kernel_api::TriMesh>,
        detail: TessellationSettings,
    ) {
        if self
            .tx
            .send(KernelRequest::MeshToSolid {
                body_id,
                mesh,
                detail,
            })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Submit a measure of a body's snapshot. One response arrives per
    /// request, tagged with the geometry revision it measured.
    pub fn request_measure(&mut self, body_id: Uuid, revision: u64, brep_blob: Arc<Vec<u8>>) {
        if self
            .tx
            .send(KernelRequest::Measure {
                body_id,
                revision,
                brep_blob,
            })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
    }

    /// Pop every response that has arrived since the last call. The caller
    /// is responsible for any document/UI bookkeeping the responses imply.
    pub fn drain(&mut self) -> Vec<KernelResponse> {
        let mut out = Vec::new();
        while let Ok(resp) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            out.push(resp);
        }
        out
    }

    /// Number of imports the worker is currently processing or has queued.
    /// Drives the bottom-panel spinner.
    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// What the kernel is doing right now, for the status bar. `None` between
    /// jobs, or before the running job has announced its first stage.
    pub fn status(&self) -> Option<String> {
        lock(&self.activity).status()
    }

    /// `(done, total)` to draw: our counted stage's when running, else the
    /// kernel's current stage's.
    pub fn progress(&self) -> Option<(u64, u64)> {
        lock(&self.activity).progress()
    }

    /// Whether a running job can be stopped — i.e. one is running at all.
    pub fn is_cancellable(&self) -> bool {
        lock(&self.activity).canceller.is_some()
    }

    /// Ask the running job to stop. It ends at the kernel's next checkpoint
    /// with a cancelled error; queued jobs are unaffected.
    pub fn cancel_current(&self) {
        if let Some(canceller) = lock(&self.activity).canceller.as_ref() {
            canceller.cancel();
        }
    }
}

/// A watch whose sink files each announced stage into the shared slot.
///
/// printCAD's own labels arrive prefixed (`kernel_ogeom::CONTEXT_PREFIX`) and
/// become the context, which persists; everything else is one of the kernel's
/// own stages and refines it.
fn watch_for(activity: &Arc<Mutex<Activity>>) -> Watch {
    let sink_activity = Arc::clone(activity);
    Watch::with_stage_sink(move |stage: kernel_ogeom::Stage<'_>| {
        let mut activity = lock(&sink_activity);
        match (
            stage.name.strip_prefix(kernel_ogeom::CONTEXT_PREFIX),
            stage.progress,
        ) {
            // Our context label: a new phase begins, everything else resets.
            (Some(ours), None) => {
                ours.clone_into(activity.context.get_or_insert_default());
                activity.detail = None;
                activity.progress = None;
                activity.own_progress = None;
            }
            // Our counted stage: takes ownership of the display.
            (Some(_), Some(counts)) => {
                activity.own_progress = Some(counts);
            }
            // The kernel's stage: informative in a sequential phase, noise
            // while our counted loop is running (many threads, one slot).
            (None, counts) => {
                if activity.own_progress.is_none() {
                    activity.detail = Some(stage.name.to_owned());
                    activity.progress = counts;
                }
            }
        }
    })
}

fn worker_loop(
    rx: Receiver<KernelRequest>,
    tx: Sender<KernelResponse>,
    activity: Arc<Mutex<Activity>>,
) {
    let mut kernel = OgeomKernel::new();
    while let Ok(request) = rx.recv() {
        let watch = watch_for(&activity);
        {
            let mut activity = lock(&activity);
            *activity = Activity {
                canceller: Some(watch.canceller()),
                ..Activity::default()
            };
        }

        let response = kernel_ogeom::watched(&watch, || match request {
            KernelRequest::ImportStep { path, detail } => {
                let started = Instant::now();
                let k0 = Instant::now();

                match kernel.import_step(&path, &detail) {
                    Ok(model) => {
                        let kernel_ms = k0.elapsed();
                        let r0 = Instant::now();
                        match std::fs::read(&path) {
                            Ok(raw_bytes) => {
                                let read_ms = r0.elapsed();
                                let worker_total = started.elapsed();
                                info!(
                                    path = %path.display(),
                                    kernel_import_ms = format!("{:.2}", kernel_ms.as_secs_f64() * 1000.0),
                                    read_asset_bytes_ms = format!("{:.2}", read_ms.as_secs_f64() * 1000.0),
                                    worker_total_ms = format!("{:.2}", worker_total.as_secs_f64() * 1000.0),
                                    "STEP BRep import worker timing (kernel thread)"
                                );
                                KernelResponse::StepImported {
                                    path,
                                    model,
                                    raw_bytes,
                                    detail,
                                    elapsed: worker_total,
                                }
                            }
                            Err(err) => KernelResponse::StepFailed {
                                path,
                                error: format!("read source bytes failed: {err}"),
                            },
                        }
                    }
                    Err(err) => KernelResponse::StepFailed {
                        path,
                        error: err.to_string(),
                    },
                }
            }
            KernelRequest::BuildSolid {
                body_id,
                ops,
                op_features,
                detail,
                preview,
            } => {
                let started = Instant::now();
                // The edited feature's ops, first to last.
                let range = preview.and_then(|feature| {
                    let first = op_features.iter().position(|f| *f == feature)?;
                    let last = op_features.iter().rposition(|f| *f == feature)?;
                    Some(first..last + 1)
                });
                match kernel.execute_solid_chain_previewing(&ops, &detail, range) {
                    Ok(result) => KernelResponse::SolidBuilt {
                        body_id,
                        result,
                        elapsed: started.elapsed(),
                    },
                    Err(err) => KernelResponse::SolidFailed {
                        body_id,
                        failed_feature: op_features.get(err.op_index).copied(),
                        error: err.message,
                    },
                }
            }
            KernelRequest::MeshToSolid {
                body_id,
                mesh,
                detail,
            } => {
                let started = Instant::now();
                match kernel.mesh_to_solid(&mesh, &detail) {
                    Ok(result) => KernelResponse::MeshSolidBuilt {
                        body_id,
                        result,
                        elapsed: started.elapsed(),
                    },
                    Err(err) => KernelResponse::MeshSolidFailed {
                        body_id,
                        error: err.to_string(),
                    },
                }
            }
            KernelRequest::Measure {
                body_id,
                revision,
                brep_blob,
            } => KernelResponse::Measured {
                body_id,
                revision,
                result: kernel
                    .physical_properties(&brep_blob)
                    .map_err(|e| e.to_string()),
            },
            KernelRequest::RepairShape {
                body_id,
                brep_blob,
                face_colors,
                detail,
            } => {
                let started = Instant::now();
                match kernel.repair_brep(&brep_blob, &face_colors, &detail) {
                    Ok(result) => KernelResponse::ShapeRepaired {
                        body_id,
                        result,
                        elapsed: started.elapsed(),
                    },
                    Err(err) => KernelResponse::RepairFailed {
                        body_id,
                        error: err.to_string(),
                    },
                }
            }
        });

        *lock(&activity) = Activity::default();
        if tx.send(response).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(context: Option<&str>, detail: Option<&str>) -> Activity {
        Activity {
            context: context.map(str::to_owned),
            detail: detail.map(str::to_owned),
            progress: None,
            own_progress: None,
            canceller: None,
        }
    }

    /// Emits a stage the way the kernel does: no printCAD prefix.
    fn ogeom_stage_for_test(name: &str, done: u64, total: u64) {
        kernel_ogeom::progress::kernel_stage_at_for_tests(name, done, total);
    }

    #[test]
    fn a_counted_stage_fills_the_bar_and_a_new_context_clears_it() {
        let slot = Arc::new(Mutex::new(Activity::default()));
        let watch = watch_for(&slot);
        kernel_ogeom::watched(&watch, || {
            kernel_ogeom::progress::context("Preparing 3 bodies");
            kernel_ogeom::progress::stage_at("bodies", 2, 3);
        });
        {
            let seen = lock(&slot);
            assert_eq!(seen.context.as_deref(), Some("Preparing 3 bodies"));
            assert_eq!(seen.progress(), Some((2, 3)), "our counts drive the bar");
            assert_eq!(
                seen.status().as_deref(),
                Some("Preparing 3 bodies"),
                "our counted stage speaks alone, no kernel detail appended"
            );
        }
        // Kernel chatter from the parallel loop must not disturb the display.
        kernel_ogeom::watched(&watch, || {
            ogeom_stage_for_test("tessellate: faces", 7, 143);
        });
        {
            let seen = lock(&slot);
            assert_eq!(
                seen.progress(),
                Some((2, 3)),
                "kernel counts do not steal the bar"
            );
            assert_eq!(seen.status().as_deref(), Some("Preparing 3 bodies"));
        }
        kernel_ogeom::watched(&watch, || {
            kernel_ogeom::progress::context("Reading STEP");
        });
        let seen = lock(&slot);
        assert_eq!(seen.context.as_deref(), Some("Reading STEP"));
        assert_eq!(seen.progress(), None, "a new context starts unknown");
    }

    #[test]
    fn a_status_line_pairs_our_label_with_the_kernels_stage() {
        assert_eq!(
            activity(Some("Fillet 4/7"), Some("boolean: intersect")).status(),
            Some("Fillet 4/7 · boolean: intersect".to_string())
        );
    }

    #[test]
    fn either_half_alone_still_reads() {
        assert_eq!(
            activity(Some("Reading STEP"), None).status(),
            Some("Reading STEP".to_string()),
            "before the kernel says anything, our own label carries the line"
        );
        assert_eq!(
            activity(None, Some("boolean: split")).status(),
            Some("boolean: split".to_string()),
            "a kernel stage with no context of ours is still worth showing"
        );
        assert_eq!(activity(None, None).status(), None, "idle shows nothing");
    }

    #[test]
    fn a_fresh_context_clears_the_stale_stage() {
        let activity = Arc::new(Mutex::new(Activity::default()));
        let watch = watch_for(&activity);
        kernel_ogeom::watched(&watch, || {
            ogeom_stage("printcad: Pad 1/2");
            ogeom_stage("boolean: intersect");
            // Moving to the next feature must not leave the previous
            // feature's stage hanging beside it.
            ogeom_stage("printcad: Fillet 2/2");
        });
        assert_eq!(
            lock(&activity).status(),
            Some("Fillet 2/2".to_string()),
            "the new feature's line starts clean"
        );
    }

    #[test]
    fn an_idle_worker_offers_nothing_to_show_or_cancel() {
        let worker = KernelWorker::spawn();
        assert_eq!(worker.status(), None);
        assert!(!worker.is_cancellable());
        // Cancelling with no job running is a no-op rather than a panic.
        worker.cancel_current();
    }

    /// Announce a stage the way the kernel does, to drive the sink under test.
    fn ogeom_stage(name: &str) {
        kernel_ogeom::stage_for_test(name);
    }
}
