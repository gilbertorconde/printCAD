//! The kernel on a browser page: workers of the page's own, each the same
//! module run as a kernel and nothing else, so the page stays live while
//! they work and several bodies build at once.
//!
//! One worker takes the requests (imports, measures, repairs) one after
//! another, as the desktop's worker thread does; the others build bodies,
//! as its build threads do, a body going to the worker that built it last
//! so its chain states are there. Jobs and answers cross as MessagePack
//! (`rmp-serde`, named fields: the payloads leave empty fields out), a
//! picked file's bytes going with the job that reads it. An import's bodies
//! come back a few at a time, so a worker never holds the whole answer
//! twice over, and its source is the page's own. A worker cannot be
//! reached while it works: Cancel stops it and starts another; one that
//! dies (out of memory, a panic) has its job answered with why, and is
//! started again.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::wasm_bindgen;

use super::{
    Activity, BuildRequest, BuildShared, KernelRequest, KernelResponse, SUPERSEDED, lock,
    serve_build, serve_request,
};
use kernel_ogeom::OgeomKernel;

/// The page's script a kernel worker starts from (web/kernel-worker.js).
const WORKER_SCRIPT: &str = "kernel-worker.js";

/// What the page asks of a worker.
#[derive(Serialize, Deserialize)]
enum Job {
    /// A request, with the bytes of the picked files it reads.
    Request {
        request: KernelRequest,
        files: Vec<(PathBuf, Vec<u8>)>,
    },
    Build(BuildRequest),
}

/// What a worker tells the page.
#[derive(Serialize, Deserialize)]
enum News {
    /// Started and listening.
    Ready,
    /// The running job's stage, for the status bar.
    Stage(Stage),
    /// Some of an import's bodies, ahead of its answer.
    Bodies(Vec<kernel_api::ImportedBody>),
    /// The job's answer; an import's carries no bodies, which came before.
    Done(KernelResponse),
    /// The worker panicked, and is about to stop.
    Panicked(String),
}

/// About how many bytes of bodies cross in one message.
const BODIES_PER_MESSAGE: usize = 64 << 20;

/// About how many bytes `body` takes on its way.
fn weight(body: &kernel_api::ImportedBody) -> usize {
    let mesh = &body.mesh;
    (mesh.positions.len() + mesh.normals.len() + mesh.colors.len()) * 12
        + (mesh.indices.len() + mesh.edges.len() + mesh.faces.len() + mesh.edge_ids.len()) * 4
        + body.brep_blob.len()
        + body.face_colors.len() * 12
}

/// A job's progress as the status bar reads it: [`Activity`] without what
/// stops the job, which stays in the worker.
#[derive(Serialize, Deserialize, Default)]
struct Stage {
    context: Option<String>,
    detail: Option<String>,
    progress: Option<(u64, u64)>,
    own_progress: Option<(u64, u64)>,
}

fn encode<T: Serialize>(value: &T) -> js_sys::Uint8Array {
    let bytes = rmp_serde::to_vec_named(value).expect("kernel messages encode");
    js_sys::Uint8Array::from(bytes.as_slice())
}

fn decode<T: for<'de> Deserialize<'de>>(data: &wasm_bindgen::JsValue) -> Option<T> {
    let bytes = js_sys::Uint8Array::new(data).to_vec();
    match rmp_serde::from_slice(&bytes) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::error!("a kernel message did not read: {err}");
            None
        }
    }
}

/// How many workers build bodies: a few, as on a desktop, never more than
/// the machine's cores beside the page's own.
pub(super) fn build_workers() -> usize {
    let cores = web_sys::window()
        .map(|w| w.navigator().hardware_concurrency() as usize)
        .unwrap_or(2);
    cores.saturating_sub(2).clamp(1, 3)
}

/// Threads each kernel worker's parallel stages may use: the machine's
/// cores shared among the workers, on a page built with them.
fn threads_per_worker() -> usize {
    if !cfg!(target_feature = "atomics") {
        return 1;
    }
    let cores = web_sys::window()
        .map(|w| w.navigator().hardware_concurrency() as usize)
        .unwrap_or(2);
    (cores / (1 + build_workers())).max(1)
}

/// One worker, as the page sees it.
struct Slot {
    worker: web_sys::Worker,
    state: Rc<RefCell<SlotState>>,
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
}

#[derive(Default)]
struct SlotState {
    ready: bool,
    /// The job out: the answers a cancel and a crash give it, and its
    /// build serial.
    busy: Option<(Failed, Option<u64>)>,
    /// An import's bodies come so far.
    bodies: Vec<kernel_api::ImportedBody>,
    /// What the worker said as it panicked.
    panic: Option<String>,
    /// The worker died, or holds memory it will never give back (a
    /// WebAssembly memory only grows): the pool starts another.
    dead: bool,
}

/// How a job is answered when it does not finish (cancelled, or its
/// worker gone), from why.
type Failed = Box<dyn Fn(String) -> KernelResponse>;

impl Slot {
    fn start(responses: &Sender<KernelResponse>, activity: &Arc<Mutex<Activity>>) -> Self {
        let options = web_sys::WorkerOptions::new();
        options.set_type(web_sys::WorkerType::Module);
        // The worker loads the same build as the page, and starts a pool of
        // threads when it has them.
        let build = if cfg!(target_feature = "atomics") {
            "threads"
        } else {
            "plain"
        };
        let script = format!(
            "{WORKER_SCRIPT}?build={build}&threads={}",
            threads_per_worker()
        );
        let worker = web_sys::Worker::new_with_options(&script, &options)
            .expect("the page starts a kernel worker");
        let state = Rc::new(RefCell::new(SlotState::default()));
        let on_message = {
            let (state, responses, activity) =
                (state.clone(), responses.clone(), Arc::clone(activity));
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                match decode::<News>(&event.data()) {
                    Some(News::Ready) => state.borrow_mut().ready = true,
                    Some(News::Stage(stage)) => {
                        let mut activity = lock(&activity);
                        activity.context = stage.context;
                        activity.detail = stage.detail;
                        activity.progress = stage.progress;
                        activity.own_progress = stage.own_progress;
                    }
                    Some(News::Bodies(bodies)) => state.borrow_mut().bodies.extend(bodies),
                    Some(News::Done(mut response)) => {
                        let mut slot = state.borrow_mut();
                        slot.busy = None;
                        let bodies = std::mem::take(&mut slot.bodies);
                        drop(slot);
                        if let KernelResponse::StepImported {
                            path,
                            model,
                            raw_bytes,
                            ..
                        } = &mut response
                        {
                            // An import grows the worker's memory to the
                            // file's size; a fresh worker gives it back.
                            state.borrow_mut().dead = true;
                            model.bodies = bodies;
                            // The source is the file the page picked.
                            if raw_bytes.is_empty() {
                                *raw_bytes = crate::platform::read(path).unwrap_or_default();
                            }
                        }
                        *lock(&activity) = Activity::default();
                        let _ = responses.send(response);
                    }
                    Some(News::Panicked(message)) => state.borrow_mut().panic = Some(message),
                    None => {}
                }
            })
        };
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        // A worker that dies (out of memory, a panic) never answers: its job
        // is answered here with why, and the pool starts another.
        let on_error = {
            let (state, responses, activity) =
                (state.clone(), responses.clone(), Arc::clone(activity));
            Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |event: web_sys::ErrorEvent| {
                let mut state = state.borrow_mut();
                state.dead = true;
                state.bodies.clear();
                let panic = state.panic.take();
                let Some((failed, _)) = state.busy.take() else {
                    return;
                };
                drop(state);
                let why = died(panic, &event.message());
                tracing::error!("{why}");
                *lock(&activity) = Activity::default();
                let _ = responses.send(failed(why));
            })
        };
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Self {
            worker,
            state,
            _on_message: on_message,
            _on_error: on_error,
        }
    }

    fn free(&self) -> bool {
        let state = self.state.borrow();
        state.ready && state.busy.is_none()
    }

    fn send(&self, job: &Job, failed: Failed, serial: Option<u64>) {
        self.state.borrow_mut().busy = Some((failed, serial));
        if let Err(err) = self.worker.post_message(&encode(job)) {
            tracing::error!("a kernel worker took no job: {err:?}");
        }
    }
}

/// The page's kernel workers and the work waiting for them.
pub(super) struct Pool {
    requests: Receiver<KernelRequest>,
    builds: Arc<Mutex<Receiver<BuildRequest>>>,
    responses: Sender<KernelResponse>,
    waiting: VecDeque<KernelRequest>,
    waiting_builds: VecDeque<BuildRequest>,
    /// The requests' worker, then the build workers, each with the slot the
    /// status bar reads.
    slots: RefCell<Vec<(Slot, Arc<Mutex<Activity>>)>>,
    /// Which build worker each body last went to.
    homes: HashMap<Uuid, usize>,
}

impl Pool {
    pub(super) fn new(
        requests: Receiver<KernelRequest>,
        builds: Arc<Mutex<Receiver<BuildRequest>>>,
        responses: Sender<KernelResponse>,
        activity: Arc<Mutex<Activity>>,
        build_activities: &[Arc<Mutex<Activity>>],
    ) -> Self {
        let slots = std::iter::once(activity)
            .chain(build_activities.iter().cloned())
            .map(|activity| (Slot::start(&responses, &activity), activity))
            .collect();
        Self {
            requests,
            builds,
            responses,
            waiting: VecDeque::new(),
            waiting_builds: VecDeque::new(),
            slots: RefCell::new(slots),
            homes: HashMap::new(),
        }
    }

    /// Hand waiting work to free workers: requests in order to the first,
    /// each build to its body's worker, or a free one for a body new to
    /// them. A build nobody needs any more is answered without going out.
    pub(super) fn pump(&mut self, shared: &BuildShared) {
        while let Ok(request) = self.requests.try_recv() {
            self.waiting.push_back(request);
        }
        while let Ok(build) = lock(&self.builds).try_recv() {
            self.waiting_builds.push_back(build);
        }
        // A worker that died is replaced before work goes out.
        for (slot, activity) in self.slots.borrow_mut().iter_mut() {
            if slot.state.borrow().dead {
                slot.worker.terminate();
                *slot = Slot::start(&self.responses, activity);
            }
        }
        let slots = self.slots.borrow();

        if slots[0].0.free()
            && let Some(request) = self.waiting.pop_front()
        {
            let files = files_read_by(&request);
            let failed = failure_of(&request);
            slots[0]
                .0
                .send(&Job::Request { request, files }, failed, None);
        }

        let mut kept = VecDeque::new();
        let mut taken: HashSet<usize> = HashSet::new();
        while let Some(build) = self.waiting_builds.pop_front() {
            if lock(&shared.book).superseded.remove(&build.serial) {
                let _ = self.responses.send(KernelResponse::SolidFailed {
                    body_id: build.body_id,
                    failed_feature: None,
                    error: SUPERSEDED.to_string(),
                    unbuilt: Vec::new(),
                    nothing_built: false,
                });
                continue;
            }
            let home = self.homes.get(&build.body_id).copied().or_else(|| {
                (1..slots.len())
                    .filter(|i| slots[*i].0.free() && !taken.contains(i))
                    .min_by_key(|i| self.homes.values().filter(|h| *h == i).count())
            });
            match home {
                Some(i) if slots[i].0.free() && !taken.contains(&i) => {
                    self.homes.insert(build.body_id, i);
                    taken.insert(i);
                    let failed = build_failure(build.body_id);
                    let serial = build.serial;
                    slots[i].0.send(&Job::Build(build), failed, Some(serial));
                }
                // Its worker is busy: it waits, ahead of later builds of
                // the same body.
                _ => kept.push_back(build),
            }
        }
        self.waiting_builds = kept;
    }

    /// Whether a worker has a job out.
    pub(super) fn any_busy(&self) -> bool {
        self.slots
            .borrow()
            .iter()
            .any(|(slot, _)| slot.state.borrow().busy.is_some())
    }

    /// Whether build `serial` is out on a worker.
    pub(super) fn is_running(&self, serial: u64) -> bool {
        self.slots.borrow().iter().any(
            |(slot, _)| matches!(&slot.state.borrow().busy, Some((_, Some(s))) if *s == serial),
        )
    }

    /// Stop every busy worker, answer its job as cancelled, and start
    /// another in its place.
    pub(super) fn cancel_busy(&self) {
        let mut slots = self.slots.borrow_mut();
        for (slot, activity) in slots.iter_mut() {
            let Some((failed, _)) = slot.state.borrow_mut().busy.take() else {
                continue;
            };
            slot.worker.terminate();
            *lock(activity) = Activity::default();
            let _ = self.responses.send(failed("cancelled".to_string()));
            *slot = Slot::start(&self.responses, activity);
        }
    }
}

/// The picked files a request reads, which a worker cannot reach itself.
fn files_read_by(request: &KernelRequest) -> Vec<(PathBuf, Vec<u8>)> {
    let path = match request {
        KernelRequest::ImportStep { path, .. } | KernelRequest::ReadSolid { path, .. } => path,
        _ => return Vec::new(),
    };
    match crate::platform::read(path) {
        Ok(bytes) => vec![(path.clone(), bytes)],
        Err(_) => Vec::new(),
    }
}

/// How a request is answered when it does not finish.
fn failure_of(request: &KernelRequest) -> Failed {
    match request {
        KernelRequest::ImportStep { path, .. } => {
            let path = path.clone();
            Box::new(move |error| KernelResponse::StepFailed {
                path: path.clone(),
                error,
            })
        }
        KernelRequest::Measure {
            body_id, revision, ..
        } => {
            let (body_id, revision) = (*body_id, *revision);
            Box::new(move |error| KernelResponse::Measured {
                body_id,
                revision,
                result: Err(error),
            })
        }
        KernelRequest::MeshToSolid { body_id, .. } => {
            let body_id = *body_id;
            Box::new(move |error| KernelResponse::MeshSolidFailed { body_id, error })
        }
        KernelRequest::MirrorShape {
            body_id,
            source_blob,
            ..
        } => {
            let (body_id, from) = (*body_id, source_blob.clone());
            Box::new(move |error| KernelResponse::ShapeMirrored {
                body_id,
                from: from.clone(),
                result: Err(error),
            })
        }
        KernelRequest::ReadSolid { body_id, asset, .. } => {
            let (body_id, asset) = (*body_id, *asset);
            Box::new(move |error| KernelResponse::SolidRead {
                body_id,
                asset,
                result: Err(error),
                elapsed: std::time::Duration::ZERO,
            })
        }
        KernelRequest::RepairShape { body_id, .. } => {
            let body_id = *body_id;
            Box::new(move |error| KernelResponse::RepairFailed { body_id, error })
        }
        KernelRequest::RefineShape { body_id, .. } => {
            let body_id = *body_id;
            Box::new(move |error| KernelResponse::ShapeRefined {
                body_id,
                result: Err(error),
                elapsed: std::time::Duration::ZERO,
            })
        }
    }
}

/// How a build is answered when it does not finish.
fn build_failure(body_id: Uuid) -> Failed {
    Box::new(move |error| KernelResponse::SolidFailed {
        body_id,
        failed_feature: None,
        error,
        unbuilt: Vec::new(),
        nothing_built: false,
    })
}

/// Why a job ended with its worker: what it said as it panicked, else
/// what the page saw.
fn died(panic: Option<String>, seen: &str) -> String {
    let why = panic.unwrap_or_else(|| seen.to_string());
    format!(
        "the kernel worker stopped ({why}); a browser gives each worker at most 4 GB, \
         which a large file can run out of: the desktop app has no such limit"
    )
}

// ---- The worker's side ----------------------------------------------------

thread_local! {
    /// Set in a kernel worker: where its news goes, and when it last told
    /// of a stage.
    static WORKER: RefCell<Option<(web_sys::DedicatedWorkerGlobalScope, web_time::Instant)>> =
        const { RefCell::new(None) };
}

/// How often a worker tells the page of its stage, at most.
const STAGE_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

fn post(scope: &web_sys::DedicatedWorkerGlobalScope, news: &News) {
    let bytes = encode(news);
    let transfer = js_sys::Array::of1(&bytes.buffer());
    if let Err(err) = scope.post_message_with_transfer(&bytes, &transfer) {
        tracing::error!("a kernel worker could not answer: {err:?}");
    }
}

/// A stage announced in a kernel worker: told to the page now and then.
pub(super) fn stage_changed(activity: &Activity) {
    WORKER.with(|worker| {
        let mut worker = worker.borrow_mut();
        let Some((scope, last)) = worker.as_mut() else {
            return;
        };
        if last.elapsed() < STAGE_EVERY {
            return;
        }
        *last = web_time::Instant::now();
        post(
            scope,
            &News::Stage(Stage {
                context: activity.context.clone(),
                detail: activity.detail.clone(),
                progress: activity.progress,
                own_progress: activity.own_progress,
            }),
        );
    });
}

// Linked for the export that starts the threads a kernel worker's pool
// runs on (`initThreadPool`), which its script calls before
// `kernel_worker_main`.
#[cfg(target_feature = "atomics")]
use wasm_bindgen_rayon as _;

/// A kernel worker's start: the page's worker script calls it once the
/// module is loaded, with the threads its pool has (1 for none). The
/// worker builds and answers until the page stops it.
#[wasm_bindgen]
pub fn kernel_worker_main(threads: usize) {
    // A panic is told to the page before the worker stops, so the job it
    // ends says why.
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        WORKER.with(|worker| {
            if let Ok(worker) = worker.try_borrow()
                && let Some((scope, _)) = worker.as_ref()
            {
                let message = info
                    .payload()
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| info.payload().downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| info.to_string());
                post(scope, &News::Panicked(message));
            }
        });
    }));
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::INFO)
            .build(),
    );
    kernel_ogeom::files::set_reader(crate::platform::read);
    // The pool the worker's script started, lent to the kernel.
    #[cfg(target_feature = "atomics")]
    if threads > 1 {
        let workers = kernel_ogeom::threads::lend_rayon();
        tracing::info!("kernel worker: {workers} threads");
    }
    #[cfg(not(target_feature = "atomics"))]
    let _ = threads;
    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    WORKER.with(|w| *w.borrow_mut() = Some((scope.clone(), web_time::Instant::now())));

    let mut kernel = OgeomKernel::new();
    let shared = BuildShared::default();
    let activity = Arc::new(Mutex::new(Activity::default()));
    let on_message = {
        let scope = scope.clone();
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Some(job) = decode::<Job>(&event.data()) else {
                return;
            };
            let mut response = match job {
                Job::Request { request, files } => {
                    let paths: Vec<PathBuf> = files.iter().map(|(p, _)| p.clone()).collect();
                    for (path, bytes) in files {
                        crate::platform::web::put(&path, bytes);
                    }
                    let response = serve_request(&mut kernel, &activity, request);
                    // Read; the page keeps the file, the worker need not.
                    for path in &paths {
                        crate::platform::web::remove(path);
                    }
                    response
                }
                Job::Build(build) => serve_build(&mut kernel, &shared, &activity, build),
            };
            // An import's bodies go a few at a time, each sent and let go
            // before the next is packed, ahead of the answer.
            if let KernelResponse::StepImported { model, .. } = &mut response {
                let mut chunk = Vec::new();
                let mut size = 0;
                for body in std::mem::take(&mut model.bodies) {
                    size += weight(&body);
                    chunk.push(body);
                    if size >= BODIES_PER_MESSAGE {
                        post(&scope, &News::Bodies(std::mem::take(&mut chunk)));
                        size = 0;
                    }
                }
                if !chunk.is_empty() {
                    post(&scope, &News::Bodies(chunk));
                }
            }
            post(&scope, &News::Done(response));
        })
    };
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The worker lives as long as the page keeps it.
    on_message.forget();
    post(&scope, &News::Ready);
}
