//! Background worker that owns the geometry kernel.
//!
//! Kernel work is fully CPU-bound (STEP parsing + tessellation). Running it
//! on the UI thread freezes the viewport for tens of seconds on big models;
//! moving it onto a dedicated worker keeps panning/orbiting smooth during
//! imports.
//!
//! The worker also performs the `std::fs::read(path)` that backs the
//! document's asset blob: that I/O belongs off the UI thread, and is
//! naturally cheap to colocate with the kernel call since the worker is
//! already off the hot path.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
use std::time::Duration;
use web_time::Instant;

use std::sync::{Arc, Mutex};

use kernel_api::{ImportedModel, Kernel, SolidBuildResult, SolidOp, TessellationSettings};
use kernel_ogeom::{Canceller, OgeomKernel, Watch};
use tracing::info;
use uuid::Uuid;

/// Job submitted from the UI thread to the kernel worker.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum KernelRequest {
    /// Read a model file and return its bodies, meshed as they are read,
    /// for the UI to register.
    ImportStep {
        path: PathBuf,
        detail: TessellationSettings,
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
    /// Mirror a source's snapshot across a plane in its own frame, for a
    /// mirrored copy.
    MirrorShape {
        body_id: Uuid,
        source_blob: Arc<Vec<u8>>,
        plane: core_document::MirrorPlane,
    },
    /// Read the first solid of a file for a body whose shape is replaced
    /// by it; the file is the worker's own temporary copy of the asset.
    ReadSolid {
        body_id: Uuid,
        asset: Uuid,
        path: PathBuf,
        detail: TessellationSettings,
    },
    /// Run the kernel's repair on an imported body's snapshot.
    RepairShape {
        body_id: Uuid,
        brep_blob: Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
        detail: TessellationSettings,
    },
    /// Rebuild a converted solid's facets on the surfaces they approximate.
    RefineShape {
        body_id: Uuid,
        brep_blob: Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
        detail: TessellationSettings,
    },
}

/// A body's build, for the build threads.
#[derive(serde::Serialize, serde::Deserialize)]
struct BuildRequest {
    body_id: Uuid,
    ops: Vec<SolidOp>,
    /// The feature each op index belongs to, so a failure is pinned on the
    /// culprit in the tree.
    op_features: Vec<Uuid>,
    detail: TessellationSettings,
    /// The feature being edited, whose preview the result carries.
    preview: Option<Uuid>,
    /// What features standing on the solid ask of it part way through.
    probes: Vec<core_document::PlanProbe>,
    /// Which build this is, for [`KernelWorker::drop_build`].
    serial: u64,
}

/// Result delivered from the worker back to the UI thread.
///
/// Each request emits exactly one response; the UI's `in_flight` counter is
/// decremented when one is drained, so spurious extras would skew the
/// status bar's busy state and job count.
/// Where a body's build stopped, and what it left out.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BuildFailure {
    pub feature: Option<Uuid>,
    pub error: String,
    pub unbuilt: Vec<Uuid>,
}

#[derive(serde::Serialize, serde::Deserialize)]
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
        /// The probes the build was asked, answered in `result.probes`.
        probes: Vec<core_document::PlanProbe>,
        /// Found among the solids kept, not built: `elapsed` says nothing
        /// of how long the body takes to build.
        kept: bool,
        /// The feature the build stopped at: the solid is the history
        /// before it.
        failed: Option<BuildFailure>,
    },
    SolidFailed {
        body_id: Uuid,
        failed_feature: Option<Uuid>,
        error: String,
        /// The features after the failing one, left out.
        unbuilt: Vec<Uuid>,
        /// The first feature failed: the body has no built solid.
        nothing_built: bool,
    },
    ShapeRepaired {
        body_id: Uuid,
        result: kernel_api::RepairResult,
        elapsed: Duration,
    },
    /// A body's new shape, read from `asset`.
    SolidRead {
        body_id: Uuid,
        asset: Uuid,
        result: Result<kernel_api::MeshSolidResult, String>,
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
    /// A converted solid refined, or why it could not be.
    ShapeRefined {
        body_id: Uuid,
        result: Result<kernel_api::MeshSolidResult, String>,
        elapsed: Duration,
    },
    /// A mirrored copy's snapshot, made from `from`.
    ShapeMirrored {
        body_id: Uuid,
        from: Arc<Vec<u8>>,
        result: Result<Vec<u8>, String>,
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
    /// The kernel's stage within that work: changes rapidly, and during a
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

/// The builds out on the build threads.
#[derive(Default)]
struct BuildBook {
    /// The builds running, by serial: when each began, and how to stop it.
    running: std::collections::HashMap<u64, (Instant, Canceller)>,
    /// Builds waiting in the queue that nobody needs, by serial: each is
    /// answered as cancelled without running.
    superseded: std::collections::HashSet<u64>,
}

/// What the build threads share: the solids kept, each body's chain
/// states, and the builds out.
#[derive(Default)]
struct BuildShared {
    built: Mutex<BuiltSolids>,
    chains: Mutex<Chains>,
    book: Mutex<BuildBook>,
}

/// How many bodies build at once: a few, each build's own work going wide
/// on the kernel's threads besides.
#[cfg(not(target_arch = "wasm32"))]
fn build_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 4).clamp(2, 4))
        .unwrap_or(2)
}

/// What a skipped build answers: a cancellation.
const SUPERSEDED: &str = "superseded by a newer plan: cancelled";

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

fn lock<T>(slot: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// UI-side handle to the worker thread. `in_flight` is incremented by every
/// `request_*` call and decremented by [`Self::drain`] for each response, so
/// the status panel can show a spinner while work is pending.
pub struct KernelWorker {
    tx: Sender<KernelRequest>,
    build_tx: Sender<BuildRequest>,
    rx: Receiver<KernelResponse>,
    in_flight: u32,
    activity: Arc<Mutex<Activity>>,
    /// Each build thread's own activity, for the status bar.
    build_activities: Vec<Arc<Mutex<Activity>>>,
    builds: Arc<BuildShared>,
    /// The serial the next build gets.
    next_serial: u64,
    /// A browser page: the worker's requests and the builds go to kernel
    /// workers of the page's own (see `pool`).
    #[cfg(target_arch = "wasm32")]
    pool: pool::Pool,
}

#[cfg(target_arch = "wasm32")]
#[path = "kernel_pool.rs"]
mod pool;

impl KernelWorker {
    /// Spawn the worker thread. The thread owns its own [`OgeomKernel`] for
    /// the lifetime of the app; the channels disconnect when the UI side is
    /// dropped, which lets the worker exit cleanly.
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = channel::<KernelRequest>();
        let (resp_tx, resp_rx) = channel::<KernelResponse>();

        let activity = Arc::new(Mutex::new(Activity::default()));
        let worker_activity = Arc::clone(&activity);

        let (build_tx, build_rx) = channel::<BuildRequest>();
        let build_rx = Arc::new(Mutex::new(build_rx));
        let builds = Arc::new(BuildShared::default());
        #[cfg(not(target_arch = "wasm32"))]
        let build_activities: Vec<_> = (0..build_threads())
            .map(|n| {
                let activity = Arc::new(Mutex::new(Activity::default()));
                let (rx, tx, shared, slot) = (
                    Arc::clone(&build_rx),
                    resp_tx.clone(),
                    Arc::clone(&builds),
                    Arc::clone(&activity),
                );
                thread::Builder::new()
                    .name(format!("printcad-build-{n}"))
                    .spawn(move || build_loop(&rx, &tx, &shared, &slot))
                    .expect("failed to spawn a build thread");
                activity
            })
            .collect();
        #[cfg(target_arch = "wasm32")]
        let build_activities: Vec<_> = (0..pool::build_workers())
            .map(|_| Arc::new(Mutex::new(Activity::default())))
            .collect();

        #[cfg(not(target_arch = "wasm32"))]
        thread::Builder::new()
            .name("printcad-kernel-worker".to_string())
            .spawn(move || worker_loop(req_rx, resp_tx, worker_activity))
            .expect("failed to spawn kernel worker thread");
        #[cfg(target_arch = "wasm32")]
        let pool = pool::Pool::new(
            req_rx,
            build_rx,
            resp_tx,
            worker_activity,
            &build_activities,
        );

        Self {
            tx: req_tx,
            build_tx,
            rx: resp_rx,
            in_flight: 0,
            activity,
            build_activities,
            builds,
            next_serial: 0,
            #[cfg(target_arch = "wasm32")]
            pool,
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

    /// Submit a body's chain. One response arrives per request. With
    /// `preview`, a feature being edited, the result also carries what that
    /// feature does (its tool, and the body without it).
    pub fn request_build_solid(
        &mut self,
        body_id: Uuid,
        ops: Vec<SolidOp>,
        op_features: Vec<Uuid>,
        detail: TessellationSettings,
        preview: Option<Uuid>,
        probes: Vec<core_document::PlanProbe>,
    ) -> u64 {
        self.next_serial += 1;
        let serial = self.next_serial;
        if self
            .build_tx
            .send(BuildRequest {
                body_id,
                ops,
                op_features,
                detail,
                preview,
                probes,
                serial,
            })
            .is_ok()
        {
            self.in_flight = self.in_flight.saturating_add(1);
        }
        serial
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

    /// Submit the refine of a converted body's solid. One response
    /// arrives per request.
    pub fn request_refine(
        &mut self,
        body_id: Uuid,
        brep_blob: Arc<Vec<u8>>,
        face_colors: Vec<[f32; 3]>,
        detail: TessellationSettings,
    ) {
        if self
            .tx
            .send(KernelRequest::RefineShape {
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

    /// Submit the reading of a body's new shape from `path`, a temporary
    /// copy of `asset` the worker removes when done. One response arrives
    /// per request.
    pub fn request_read_solid(
        &mut self,
        body_id: Uuid,
        asset: Uuid,
        path: PathBuf,
        detail: TessellationSettings,
    ) {
        if self
            .tx
            .send(KernelRequest::ReadSolid {
                body_id,
                asset,
                path,
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

    /// Submit the mirror of a source's snapshot for a mirrored copy. One
    /// response arrives per request.
    pub fn request_mirror(
        &mut self,
        body_id: Uuid,
        source_blob: Arc<Vec<u8>>,
        plane: core_document::MirrorPlane,
    ) {
        if self
            .tx
            .send(KernelRequest::MirrorShape {
                body_id,
                source_blob,
                plane,
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
        // A browser page hands waiting work to its free kernel workers.
        #[cfg(target_arch = "wasm32")]
        self.pool.pump(&self.builds);
        let mut out = Vec::new();
        while let Ok(resp) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            out.push(resp);
        }
        out
    }

    /// Number of requests the worker threads are processing or have queued.
    /// Drives the bottom-panel spinner.
    pub fn in_flight(&self) -> u32 {
        self.in_flight
    }

    /// What the kernel is doing right now, for the status bar. `None` between
    /// jobs, or before the running job has announced its first stage.
    pub fn status(&self) -> Option<String> {
        self.activities().find_map(|a| lock(a).status())
    }

    /// The worker's activity, then each build thread's.
    fn activities(&self) -> impl Iterator<Item = &Arc<Mutex<Activity>>> {
        std::iter::once(&self.activity).chain(&self.build_activities)
    }

    /// `(done, total)` to draw: our counted stage's when running, else the
    /// kernel's current stage's.
    pub fn progress(&self) -> Option<(u64, u64)> {
        self.activities().find_map(|a| lock(a).progress())
    }

    /// Whether a running job can be stopped, that is, whether one is running.
    pub fn is_cancellable(&self) -> bool {
        // A page's worker keeps its job's canceller; a busy one is stopped.
        #[cfg(target_arch = "wasm32")]
        if self.pool.any_busy() {
            return true;
        }
        self.activities().any(|a| lock(a).canceller.is_some())
    }

    /// A newer plan replaced the body's build: the build is dropped. One
    /// still in the queue is answered as cancelled without running; one
    /// running stops at the kernel's next checkpoint, unless it has run
    /// past half of `usual` (the body's last build time), so that a stream
    /// of edits still shows a shape now and then. Whether it was dropped.
    pub fn drop_build(&self, serial: u64, usual: Option<Duration>) -> bool {
        // A page's kernel worker cannot be reached while it builds.
        #[cfg(target_arch = "wasm32")]
        if self.pool.is_running(serial) {
            let _ = usual;
            return false;
        }
        let mut book = lock(&self.builds.book);
        match book.running.get(&serial) {
            Some((since, canceller)) => {
                if usual.is_some_and(|usual| since.elapsed() * 2 > usual) {
                    return false;
                }
                canceller.cancel();
                true
            }
            None => {
                book.superseded.insert(serial);
                true
            }
        }
    }

    /// Ask the running jobs to stop. Each ends at the kernel's next
    /// checkpoint with a cancelled error; queued jobs are unaffected.
    pub fn cancel_current(&self) {
        // A page's kernel workers hear nothing while they work: the busy
        // ones are stopped and started afresh.
        #[cfg(target_arch = "wasm32")]
        self.pool.cancel_busy();
        for activity in self.activities() {
            if let Some(canceller) = lock(activity).canceller.as_ref() {
                canceller.cancel();
            }
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
        // In a page's kernel worker, the page's status bar hears of it.
        #[cfg(target_arch = "wasm32")]
        pool::stage_changed(&activity);
    })
}

/// How many built solids the worker keeps per body, and in all.
const KEPT_PER_BODY: usize = 8;
const KEPT: usize = 48;

/// Solids built lately, each with the body and what it was built from:
/// moving a body's tip back and forth through its history, or undoing and
/// redoing, finds each solid built already.
#[derive(Default)]
struct BuiltSolids {
    kept: std::collections::VecDeque<(Uuid, u64, SolidBuildResult)>,
}

impl BuiltSolids {
    /// What a build is made from, as one number: the ops, the features
    /// they build, the detail, the preview and the probes.
    fn key(
        ops: &[SolidOp],
        op_features: &[Uuid],
        detail: &TessellationSettings,
        preview: &Option<std::ops::Range<usize>>,
        asked: &[kernel_api::ChainProbe],
    ) -> Option<u64> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_vec(&(ops, op_features, detail, preview, asked))
            .ok()?
            .hash(&mut hasher);
        Some(hasher.finish())
    }

    fn get(&mut self, body: Uuid, key: u64) -> Option<SolidBuildResult> {
        let at = self
            .kept
            .iter()
            .position(|(b, k, _)| *b == body && *k == key)?;
        // The newest used last goes to the back, the last to be dropped.
        let entry = self.kept.remove(at)?;
        let result = entry.2.clone();
        self.kept.push_back(entry);
        Some(result)
    }

    fn keep(&mut self, body: Uuid, key: u64, result: &SolidBuildResult) {
        self.kept.retain(|(b, k, _)| !(*b == body && *k == key));
        self.kept.push_back((body, key, result.clone()));
        if self.kept.iter().filter(|(b, ..)| *b == body).count() > KEPT_PER_BODY
            && let Some(oldest) = self.kept.iter().position(|(b, ..)| *b == body)
        {
            self.kept.remove(oldest);
        }
        while self.kept.len() > KEPT {
            self.kept.pop_front();
        }
    }
}

/// How many bodies keep their chain's states between builds.
const CHAINS_KEPT: usize = 12;

/// What each body's builds keep between them (`kernel_ogeom::ChainCache`),
/// for the bodies built last.
#[derive(Default)]
struct Chains {
    /// Most recently built last.
    bodies: std::collections::VecDeque<(Uuid, kernel_ogeom::ChainCache)>,
}

impl Chains {
    /// The body's cache, taken out while its build runs; a fresh one when
    /// it has none.
    fn take(&mut self, body: Uuid) -> kernel_ogeom::ChainCache {
        match self.bodies.iter().position(|(b, _)| *b == body) {
            Some(at) => self.bodies.remove(at).map(|(_, c)| c).unwrap_or_default(),
            None => kernel_ogeom::ChainCache::default(),
        }
    }

    /// Put the body's cache back after its build; the body built longest
    /// ago gives its up when too many keep one.
    fn put(&mut self, body: Uuid, cache: kernel_ogeom::ChainCache) {
        self.bodies.push_back((body, cache));
        while self.bodies.len() > CHAINS_KEPT {
            self.bodies.pop_front();
        }
    }
}

/// A build thread: takes the next build from the queue, one at a time.
#[cfg(not(target_arch = "wasm32"))]
fn build_loop(
    rx: &Mutex<Receiver<BuildRequest>>,
    tx: &Sender<KernelResponse>,
    shared: &BuildShared,
    activity: &Arc<Mutex<Activity>>,
) {
    let mut kernel = OgeomKernel::new();
    loop {
        let Ok(request) = lock(rx).recv() else {
            return;
        };
        let response = serve_build(&mut kernel, shared, activity, request);
        if tx.send(response).is_err() {
            return;
        }
    }
}

/// One build on a build thread, watched as a worker request is.
fn serve_build(
    kernel: &mut OgeomKernel,
    shared: &BuildShared,
    activity: &Arc<Mutex<Activity>>,
    request: BuildRequest,
) -> KernelResponse {
    let watch = watch_for(activity);
    *lock(activity) = Activity {
        canceller: Some(watch.canceller()),
        ..Activity::default()
    };
    let serial = request.serial;
    let response = kernel_ogeom::watched(&watch, || build(kernel, shared, &watch, request));
    lock(&shared.book).running.remove(&serial);
    *lock(activity) = Activity::default();
    response
}

/// Build one body's solid: from the solids kept when it was built alike
/// before, else through the body's chain states.
fn build(
    kernel: &mut OgeomKernel,
    shared: &BuildShared,
    watch: &Watch,
    request: BuildRequest,
) -> KernelResponse {
    let BuildRequest {
        body_id,
        ops,
        op_features,
        detail,
        preview,
        probes,
        serial,
    } = request;
    let started = Instant::now();
    {
        let mut book = lock(&shared.book);
        if book.superseded.remove(&serial) {
            return KernelResponse::SolidFailed {
                body_id,
                failed_feature: None,
                error: SUPERSEDED.to_string(),
                unbuilt: Vec::new(),
                nothing_built: false,
            };
        }
        book.running.insert(serial, (started, watch.canceller()));
    }
    // The edited feature's ops, first to last.
    let range = preview.and_then(|feature| {
        let first = op_features.iter().position(|f| *f == feature)?;
        let last = op_features.iter().rposition(|f| *f == feature)?;
        Some(first..last + 1)
    });
    let outcome = run_chain(
        kernel,
        shared,
        body_id,
        Chain {
            ops: &ops,
            op_features: &op_features,
            detail: &detail,
            range,
            probes: &probes,
        },
    );
    match outcome {
        Ok((result, kept)) => KernelResponse::SolidBuilt {
            body_id,
            result,
            elapsed: started.elapsed(),
            probes,
            kept,
            failed: None,
        },
        Err(err) => {
            let failed_feature = op_features.get(err.op_index).copied();
            let cancelled = err.message.contains("cancelled");
            let start = failed_feature.and_then(|f| op_features.iter().position(|o| *o == f));
            let unbuilt: Vec<Uuid> = match start {
                Some(start) => {
                    let mut after: Vec<Uuid> = Vec::new();
                    for f in &op_features[start..] {
                        if Some(*f) != failed_feature && !after.contains(f) {
                            after.push(*f);
                        }
                    }
                    after
                }
                None => Vec::new(),
            };
            // The history before the failing feature is what the body is.
            match start {
                Some(start) if start > 0 && !cancelled => {
                    let probes: Vec<core_document::PlanProbe> = probes
                        .into_iter()
                        .filter(|p| p.probe.after_op <= start)
                        .collect();
                    match run_chain(
                        kernel,
                        shared,
                        body_id,
                        Chain {
                            ops: &ops[..start],
                            op_features: &op_features[..start],
                            detail: &detail,
                            range: None,
                            probes: &probes,
                        },
                    ) {
                        Ok((result, kept)) => KernelResponse::SolidBuilt {
                            body_id,
                            result,
                            elapsed: started.elapsed(),
                            probes,
                            kept,
                            failed: Some(BuildFailure {
                                feature: failed_feature,
                                error: err.message,
                                unbuilt,
                            }),
                        },
                        Err(_) => KernelResponse::SolidFailed {
                            body_id,
                            failed_feature,
                            error: err.message,
                            unbuilt,
                            nothing_built: false,
                        },
                    }
                }
                _ => KernelResponse::SolidFailed {
                    body_id,
                    failed_feature,
                    error: err.message,
                    unbuilt,
                    nothing_built: start == Some(0) && !cancelled,
                },
            }
        }
    }
}

/// What one chain run builds: the ops, the feature of each, the detail,
/// the edited feature's ops and the probes asked.
struct Chain<'a> {
    ops: &'a [kernel_api::SolidOp],
    op_features: &'a [Uuid],
    detail: &'a TessellationSettings,
    range: Option<std::ops::Range<usize>>,
    probes: &'a [core_document::PlanProbe],
}

/// One chain run, from the solids kept when it was built alike before,
/// else through the body's chain states; whether it was found kept.
fn run_chain(
    kernel: &mut OgeomKernel,
    shared: &BuildShared,
    body_id: Uuid,
    chain: Chain<'_>,
) -> Result<(SolidBuildResult, bool), kernel_api::ChainError> {
    let Chain {
        ops,
        op_features,
        detail,
        range,
        probes,
    } = chain;
    let asked: Vec<kernel_api::ChainProbe> = probes.iter().map(|p| p.probe).collect();
    // Each op's faces are named after the feature it builds.
    let tags: Vec<kernel_api::TopoName> = op_features
        .iter()
        .map(|f| kernel_api::naming::name_of_id(f.as_bytes()))
        .collect();
    let key = BuiltSolids::key(ops, op_features, detail, &range, &asked);
    if let Some(result) = key.and_then(|key| lock(&shared.built).get(body_id, key)) {
        return Ok((result, true));
    }
    let mut states = lock(&shared.chains).take(body_id);
    let outcome =
        kernel.execute_solid_chain_cached(ops, &tags, detail, range, &asked, Some(&mut states));
    lock(&shared.chains).put(body_id, states);
    if let (Ok(result), Some(key)) = (&outcome, key) {
        lock(&shared.built).keep(body_id, key, result);
    }
    outcome.map(|result| (result, false))
}

#[cfg(not(target_arch = "wasm32"))]
fn worker_loop(
    rx: Receiver<KernelRequest>,
    tx: Sender<KernelResponse>,
    activity: Arc<Mutex<Activity>>,
) {
    let mut kernel = OgeomKernel::new();
    while let Ok(request) = rx.recv() {
        let response = serve_request(&mut kernel, &activity, request);
        if tx.send(response).is_err() {
            return;
        }
    }
}

/// One request on the worker: watched, so the status bar shows its stages
/// and Cancel reaches it.
fn serve_request(
    kernel: &mut OgeomKernel,
    activity: &Arc<Mutex<Activity>>,
    request: KernelRequest,
) -> KernelResponse {
    {
        let watch = watch_for(activity);
        {
            let mut activity = lock(activity);
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
                        // A page's worker sends no source back: the page
                        // picked the file and holds it.
                        let source = if cfg!(target_arch = "wasm32") {
                            Ok(Vec::new())
                        } else {
                            crate::platform::read(&path)
                        };
                        match source {
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
            KernelRequest::MirrorShape {
                body_id,
                source_blob,
                plane,
            } => {
                use kernel_api::KernelQueries;
                let result = kernel_ogeom::QUERIES
                    .mirror(
                        &source_blob,
                        plane.point.map(f64::from),
                        plane.normal.map(f64::from),
                    )
                    .map_err(|e| e.to_string());
                KernelResponse::ShapeMirrored {
                    body_id,
                    from: source_blob,
                    result,
                }
            }
            KernelRequest::ReadSolid {
                body_id,
                asset,
                path,
                detail,
            } => {
                let started = Instant::now();
                let result = kernel.read_solid(&path, &detail).map_err(|e| e.to_string());
                // The worker's own copy of the asset.
                let _ = std::fs::remove_file(&path);
                KernelResponse::SolidRead {
                    body_id,
                    asset,
                    result,
                    elapsed: started.elapsed(),
                }
            }
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
            KernelRequest::RefineShape {
                body_id,
                brep_blob,
                face_colors,
                detail,
            } => {
                let started = Instant::now();
                KernelResponse::ShapeRefined {
                    body_id,
                    result: kernel
                        .refine_brep(&brep_blob, &face_colors, &detail)
                        .map_err(|e| e.to_string()),
                    elapsed: started.elapsed(),
                }
            }
        });

        *lock(activity) = Activity::default();
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_solid_built_once_is_found_again_by_what_it_was_built_from() {
        let mut built = BuiltSolids::default();
        let body = Uuid::new_v4();
        let detail = TessellationSettings::default();
        let key = |ops: &[SolidOp]| BuiltSolids::key(ops, &[], &detail, &None, &[]).unwrap();
        let refine = [SolidOp::Refine];
        let result = SolidBuildResult {
            brep_blob: b"solid".to_vec(),
            ..Default::default()
        };
        built.keep(body, key(&refine), &result);
        assert_eq!(built.get(body, key(&refine)).unwrap().brep_blob, b"solid");
        assert!(built.get(body, key(&[])).is_none(), "other ops, not built");
        assert!(
            built.get(Uuid::new_v4(), key(&refine)).is_none(),
            "another body"
        );
        // A body keeps its latest few.
        for n in 0..KEPT_PER_BODY as u64 + 2 {
            built.keep(body, 1000 + n, &result);
        }
        assert_eq!(built.kept.len(), KEPT_PER_BODY);
        assert!(built.get(body, key(&refine)).is_none(), "the oldest went");
    }

    fn activity(context: Option<&str>, detail: Option<&str>) -> Activity {
        Activity {
            context: context.map(str::to_owned),
            detail: detail.map(str::to_owned),
            progress: None,
            own_progress: None,
            ..Activity::default()
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

    /// A plate with many holes cut one by one: a build that takes a while.
    fn slow_chain() -> Vec<SolidOp> {
        use kernel_api::{BooleanOp, Placement, PrimitiveKind};
        let mut ops = vec![SolidOp::Primitive {
            kind: PrimitiveKind::Box {
                length: 200.0,
                width: 200.0,
                height: 5.0,
            },
            placement: Placement::default(),
            op: BooleanOp::NewSolid,
        }];
        for i in 0..40 {
            ops.push(SolidOp::Primitive {
                kind: PrimitiveKind::Cylinder {
                    radius: 2.0,
                    height: 10.0,
                    angle_deg: 360.0,
                },
                placement: Placement {
                    origin: [
                        5.0 + (i % 15) as f64 * 13.0,
                        5.0 + (i / 15) as f64 * 13.0,
                        -1.0,
                    ],
                    ..Placement::default()
                },
                op: BooleanOp::Cut,
            });
        }
        ops
    }

    /// Every answer the worker gives until `count` have come.
    fn answers(worker: &mut KernelWorker, count: usize) -> Vec<KernelResponse> {
        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(120);
        while out.len() < count && Instant::now() < deadline {
            out.extend(worker.drain());
            std::thread::sleep(Duration::from_millis(5));
        }
        out
    }

    fn cancelled(response: &KernelResponse) -> bool {
        matches!(response, KernelResponse::SolidFailed { error, .. } if error.contains("cancelled"))
    }

    /// A build dropped while it waits behind others is answered as
    /// cancelled without running; the ones running are not touched.
    #[test]
    fn a_dropped_build_waiting_in_the_queue_never_runs() {
        let mut worker = KernelWorker::spawn();
        let detail = TessellationSettings::default();
        let busy = worker.build_activities.len();
        let bodies: Vec<Uuid> = (0..busy).map(|_| Uuid::new_v4()).collect();
        for body in &bodies {
            worker.request_build_solid(
                *body,
                slow_chain(),
                Vec::new(),
                detail.clone(),
                None,
                Vec::new(),
            );
        }
        let last = Uuid::new_v4();
        let waiting =
            worker.request_build_solid(last, slow_chain(), Vec::new(), detail, None, Vec::new());
        assert!(worker.drop_build(waiting, None));
        let out = answers(&mut worker, busy + 1);
        let built = out
            .iter()
            .filter(|r| matches!(r, KernelResponse::SolidBuilt { body_id, .. } if bodies.contains(body_id)))
            .count();
        assert_eq!(built, busy, "the builds running land");
        assert!(
            out.iter().any(
                |r| matches!(r, KernelResponse::SolidFailed { body_id, error, .. }
                if *body_id == last && error == SUPERSEDED)
            ),
            "the dropped one is skipped"
        );
    }

    /// A box, a step that cannot build, and a second box: the body is the
    /// first box, the failure is on the second step, and the third is
    /// left out.
    #[test]
    fn a_failing_step_leaves_the_history_before_it() {
        use kernel_api::{BooleanOp, Placement, PrimitiveKind, SurfaceOp};
        let cube = |at: f64| SolidOp::Primitive {
            kind: PrimitiveKind::Box {
                length: 10.0,
                width: 10.0,
                height: 10.0,
            },
            placement: Placement {
                origin: [at, 0.0, 0.0],
                ..Placement::default()
            },
            op: if at == 0.0 {
                BooleanOp::NewSolid
            } else {
                BooleanOp::Fuse
            },
        };
        let broken = SolidOp::Surface(SurfaceOp::PlanarFill { curves: Vec::new() });
        let [a, b, c] = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let mut worker = KernelWorker::spawn();
        let detail = TessellationSettings::default();
        worker.request_build_solid(
            Uuid::new_v4(),
            vec![cube(0.0), broken.clone(), cube(20.0)],
            vec![a, b, c],
            detail.clone(),
            None,
            Vec::new(),
        );
        let out = answers(&mut worker, 1);
        let KernelResponse::SolidBuilt {
            result,
            failed: Some(failed),
            ..
        } = &out[0]
        else {
            panic!("the history before the failure builds");
        };
        assert_eq!(failed.feature, Some(b));
        assert_eq!(failed.unbuilt, vec![c]);
        let span = result.mesh.bounds().map(|(lo, hi)| hi[0] - lo[0]);
        assert!(span.is_some_and(|x| (x - 10.0).abs() < 1e-3), "{span:?}");

        worker.request_build_solid(
            Uuid::new_v4(),
            vec![broken, cube(20.0)],
            vec![b, c],
            detail,
            None,
            Vec::new(),
        );
        let out = answers(&mut worker, 1);
        assert!(
            matches!(&out[0], KernelResponse::SolidFailed { nothing_built: true, unbuilt, .. }
                if *unbuilt == vec![c]),
            "a failing first step leaves nothing built"
        );
    }

    /// Two bodies build at the same time.
    #[test]
    fn two_bodies_build_at_once() {
        let mut worker = KernelWorker::spawn();
        let detail = TessellationSettings::default();
        for _ in 0..2 {
            worker.request_build_solid(
                Uuid::new_v4(),
                slow_chain(),
                Vec::new(),
                detail.clone(),
                None,
                Vec::new(),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut together = false;
        while !together && Instant::now() < deadline {
            together = lock(&worker.builds.book).running.len() == 2;
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(together, "both ran at once");
        assert_eq!(answers(&mut worker, 2).len(), 2);
    }

    /// A build dropped while it runs stops; one past half its body's usual
    /// time is left to finish.
    #[test]
    fn a_dropped_build_running_stops_unless_nearly_done() {
        let mut worker = KernelWorker::spawn();
        let detail = TessellationSettings::default();
        let body = Uuid::new_v4();
        let running = |worker: &KernelWorker| !lock(&worker.builds.book).running.is_empty();
        let serial = worker.request_build_solid(
            body,
            slow_chain(),
            Vec::new(),
            detail.clone(),
            None,
            Vec::new(),
        );
        while !running(&worker) {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(worker.drop_build(serial, None));
        assert!(cancelled(&answers(&mut worker, 1)[0]), "it stopped");

        let serial =
            worker.request_build_solid(body, slow_chain(), Vec::new(), detail, None, Vec::new());
        while !running(&worker) {
            std::thread::sleep(Duration::from_millis(1));
        }
        std::thread::sleep(Duration::from_millis(20));
        assert!(
            !worker.drop_build(serial, Some(Duration::from_millis(10))),
            "past half of its usual time: it finishes"
        );
        let out = answers(&mut worker, 1);
        assert!(matches!(&out[0], KernelResponse::SolidBuilt { .. }));
    }
}
