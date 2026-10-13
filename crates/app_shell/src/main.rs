// A release build on Windows is a windowed program: no console opens beside
// it. `--mcp` still speaks over the pipes its agent hands it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod camera;
mod console;
#[cfg(not(target_arch = "wasm32"))]
mod headless;
mod kernel_worker;
mod log_panel;
mod orientation_cube;
mod platform;
mod proof;
mod script_library;
mod thumbnail;
mod ui;

#[cfg(not(target_arch = "wasm32"))]
use anyhow::{Context, Result};
use app::doc_io::FileDialogResult;
use core_document::{Document, DocumentService, WorkbenchId};
use kernel_api::TessellationSettings;
use kernel_worker::KernelWorker;
use log_panel as app_log;
#[cfg(not(target_arch = "wasm32"))]
use render_wgpu::RenderBackend;
use render_wgpu::{FrameSubmission, RenderSettings, Renderer};
use settings::{SettingsStore, UserSettings};
use std::path::PathBuf;

use tracing::error;
#[cfg(not(target_arch = "wasm32"))]
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use ui::{ActiveWorkbench, Screen, UiLayer};
use uuid::Uuid;
use web_time::Instant;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{WindowAttributes, WindowId},
};
use workbenches::register_all_workbenches;

#[cfg(not(target_arch = "wasm32"))]
fn init_tracing_subscriber() -> anyhow::Result<Option<tracing_appender::non_blocking::WorkerGuard>>
{
    let console_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    if let Ok(raw) = std::env::var("PRINTCAD_CAMERA_LOG") {
        let path = PathBuf::from(raw.trim());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("PRINTCAD_CAMERA_LOG={}", path.display()))?;

        let (non_blocking, guard) = tracing_appender::non_blocking(file);

        let cam_filter = std::env::var("PRINTCAD_CAMERA_LOG_FILTER")
            .unwrap_or_else(|_| "printcad.camera=trace".to_string());
        let file_layer = fmt::layer()
            .with_ansi(false)
            .with_writer(non_blocking)
            .with_filter(EnvFilter::new(cam_filter.trim()));

        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_filter(console_filter.clone()),
            )
            .with(file_layer)
            .init();

        Ok(Some(guard))
    } else {
        tracing_subscriber::registry()
            .with(
                fmt::layer()
                    .with_writer(std::io::stderr)
                    .with_filter(console_filter),
            )
            .init();

        Ok(None)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<()> {
    core_document::set_writer_version(env!("CARGO_PKG_VERSION"));
    let _camera_tracing_guard =
        init_tracing_subscriber().context("tracing subscriber init failed")?;

    let mut registry = DocumentService::default();
    registry.set_kernel(&kernel_ogeom::QUERIES);
    register_all_workbenches(&mut registry)?;

    app_log::info(format!(
        "Registered {} workbenches",
        registry.workbench_descriptors().count()
    ));

    let settings_store = SettingsStore::new().context("settings store init failed")?;
    let user_settings = loaded_settings(&settings_store);

    // `--mcp` and a script run from the command line both finish before
    // any window opens.
    let words: Vec<String> = std::env::args().skip(1).collect();
    // `printcad --mcp`: an agent's MCP server, relayed to the running app.
    if let Some(relayed) = app::mcp::relay_from_args(&words) {
        if let Err(err) = relayed {
            eprintln!("{err}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let packages = app::packages::register(&mut registry, &user_settings.packages);
    let user_settings_check_updates = user_settings.packages.check_updates;

    // The benches' own settings, back from the file.
    registry.apply_settings(&user_settings.workbenches);
    match headless::parse(&words) {
        Ok(Some(invocation)) => {
            let finished = headless::run(&invocation, registry)?;
            std::process::exit(if finished { 0 } else { 1 });
        }
        Ok(None) => {}
        Err(usage) => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }

    let event_loop = EventLoop::<AppEvent>::with_user_event()
        .build()
        .context("failed to create event loop")?;
    let render_settings = RenderSettings {
        preferred_gpu: user_settings.preferred_gpu.clone(),
        msaa_samples: user_settings.rendering.msaa_samples,
        ..RenderSettings::default()
    };
    let mut app = PrintCadApp::new(
        render_settings,
        settings_store,
        user_settings,
        registry,
        event_loop.create_proxy(),
    );
    app.packages = packages;
    // `printcad <file>…`, as a desktop entry or file manager starts it.
    app.launch_files = words
        .iter()
        .filter(|word| !word.starts_with('-'))
        .map(PathBuf::from)
        .collect();
    if user_settings_check_updates {
        app.check_package_updates();
        // The stores' lists, for whoever has a package one took off.
        app.look_at_stores(true);
    }
    if app.user_settings.updates.check_at_start {
        app.check_app_release(true);
    }
    app.start_agent_server();
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

/// The user's settings from `store`, the defaults when they cannot be
/// read, with ids that were renamed since read as their new names.
fn loaded_settings(store: &SettingsStore) -> UserSettings {
    let mut user_settings = match store.load() {
        Ok(settings) => settings,
        Err(err) => {
            app_log::warn(format!("Using default settings (failed to load): {err}"));
            UserSettings::default()
        }
    };
    user_settings.rename_ids(&settings::Renames {
        workbench: &|id| core_document::renamed::workbench(id).to_owned(),
        command: &|id| core_document::renamed::command(id).into_owned(),
        group: &|id| core_document::renamed::workbench_prefixed(id).into_owned(),
    });
    user_settings
}

/// A browser page: the same app, started by the page's script. Logs go to
/// the browser's console, a panic says there what failed, and imports read
/// the files the user picked. No command line, no packages, no agents: what
/// needs a desktop says so where it is asked for.
#[cfg(target_arch = "wasm32")]
fn main() {
    use winit::platform::web::EventLoopExtWebSys;
    // In a kernel worker the module only serves the kernel
    // (`kernel_worker_main`); the app is the page's.
    if web_sys::window().is_none() {
        return;
    }
    console_error_panic_hook::set_once();
    core_document::set_writer_version(env!("CARGO_PKG_VERSION"));
    tracing_wasm::set_as_global_default_with_config(
        tracing_wasm::WASMLayerConfigBuilder::new()
            .set_max_level(tracing::Level::INFO)
            .build(),
    );
    kernel_ogeom::files::set_reader(platform::read);

    let mut registry = DocumentService::default();
    registry.set_kernel(&kernel_ogeom::QUERIES);
    if let Err(err) = register_all_workbenches(&mut registry) {
        error!("the workbenches did not register: {err}");
        return;
    }
    let settings_store = match SettingsStore::new() {
        Ok(store) => store,
        Err(err) => {
            error!("the settings are unavailable: {err}");
            return;
        }
    };
    let user_settings = loaded_settings(&settings_store);
    registry.apply_settings(&user_settings.workbenches);

    let event_loop = match EventLoop::<AppEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            error!("the page's event loop did not start: {err}");
            return;
        }
    };
    let render_settings = RenderSettings {
        preferred_gpu: user_settings.preferred_gpu.clone(),
        msaa_samples: user_settings.rendering.msaa_samples,
        ..RenderSettings::default()
    };
    let check_stores = user_settings.packages.check_updates;
    platform::web::guard_unsaved();
    // The files the page kept (documents, autosaved copies) are read back
    // first, so the start page finds them.
    wasm_bindgen_futures::spawn_local(async move {
        platform::web::load_kept().await;
        let mut app = PrintCadApp::new(
            render_settings,
            settings_store,
            user_settings,
            registry,
            event_loop.create_proxy(),
        );
        app.start_page_packages();
        if check_stores {
            app.look_at_stores(true);
        }
        event_loop.spawn_app(app);
    });
}

/// What a background thread needs the event loop to notice.
///
/// The loop renders on demand and sleeps in between, so a thread whose work
/// produces no window event has to knock on the door itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEvent {
    /// The 6-DoF mouse moved or a button changed.
    DeviceInput,
    /// The script thread asked for something or printed.
    Script,
    /// An agent called a tool, or a chat has news.
    Agent,
    /// A browser page's renderer is ready: the window can draw.
    #[cfg(target_arch = "wasm32")]
    GfxReady,
}

/// Where a re-derived remote import's meshes belong.
pub(crate) struct RemoteImportRoute {
    pub body_ids: Vec<core_document::BodyId>,
    pub asset_id: uuid::Uuid,
}

/// Bounds of a body mesh, keyed by body and mesh revision.
type DimensionCache = (core_document::BodyId, u64, ([f32; 3], [f32; 3]));

struct PrintCadApp {
    settings: RenderSettings,
    /// The active tab: the document on screen and everything the app keeps
    /// about it.
    session: app::session::DocumentSession,
    /// Every tab in strip order; the active one's slot is parked `None`
    /// because its session is `self.session`.
    tabs: Vec<app::session::TabSlot>,
    active_tab: usize,
    /// Which tab asked for each STEP import in flight, by the path the
    /// kernel worker echoes back.
    import_owner: std::collections::HashMap<PathBuf, Uuid>,
    frame_submission: FrameSubmission,
    /// Window + renderer + UI layer; teardown order is enforced by the
    /// [`app::Gfx`] struct's field order (renderer before window).
    gfx: Option<app::Gfx>,
    settings_store: SettingsStore,
    user_settings: UserSettings,
    last_frame_time: Option<Instant>,
    current_fps: f32,
    gpu_name: Option<String>,
    graphics_api: Option<&'static str>,
    available_gpus: Vec<String>,
    fps_accum_time: f32,
    fps_frame_count: u32,
    cursor_in_viewport: Option<(f32, f32)>,
    registry: DocumentService,
    /// Recently opened documents and the last dialog directory.
    recent: settings::recent::RecentStore,
    // Pending file dialog result from background thread.
    file_dialog_rx: Option<std::sync::mpsc::Receiver<FileDialogResult>>,
    /// An export being written on its own thread.
    export_rx: Option<std::sync::mpsc::Receiver<crate::app::export::ExportOutcome>>,
    /// The export options last confirmed, which the dialog opens on.
    last_export: crate::app::export::ExportDraft,
    // Background worker that owns the geometry kernel. STEP imports run there
    // so the viewport stays interactive while a multi-million-tri model is
    // tessellated; responses are drained once per frame in `about_to_wait`.
    kernel_worker: KernelWorker,
    /// Background reader for a 6-DoF mouse. It holds the puck's current
    /// deflection; the frame loop integrates it.
    nav_device: app::sixdof::SixDofWorker,
    /// Dev/bench hook: `PRINTCAD_OPEN_FILE` triggers one STEP import at
    /// startup, so a benchmark run needs no dialog interaction.
    bench_open_fired: bool,
    /// Whether `PRINTCAD_BENCH_SELECT` has fired.
    bench_select_fired: bool,
    /// Frames the `PRINTCAD_BENCH_PICTURE` hook has waited for the scene
    /// to settle; past its wait, the picture has been asked for.
    bench_picture_frames: u32,
    /// `PRINTCAD_BENCH_TASK` has opened its task.
    bench_task_fired: bool,
    /// `PRINTCAD_BENCH_WORKBENCH` has switched to its bench.
    bench_workbench_fired: bool,
    bench_repair_fired: bool,
    bench_convert_fired: bool,
    bench_refine_fired: bool,
    /// Frames since the bench click hook saw geometry; drives its stages.
    bench_click_frames: u32,
    /// Process start, for the `PRINTCAD_EXIT_AFTER_MS` bench hook.
    bench_started: Instant,
    /// Rolling per-phase frame cost, emitted once a second alongside the FPS
    /// counter (target `printcad.frame`): (ui ms, render ms, frames).
    frame_phase_accum: (f32, f32, u32),
    /// Scene redraws in the current 1 s window and the last completed one.
    /// The scene is cached between changes, so this diverges from fps: it
    /// is what "the parts' fps" actually is.
    scene_redraw_accum: u32,
    scene_redraws_per_s: u32,
    /// How often the status-bar text changed this second, and what it last
    /// read. A readable status changes a few times a second at most; a slot
    /// being overwritten by twenty threads changes every frame.
    status_changes_accum: u32,
    last_status_text: Option<String>,
    /// When user input last arrived. Frames keep coming for a short tail
    /// after input so egui reactions and pick readbacks land, then the loop
    /// sleeps until the next event (render on demand).
    last_input_time: Option<Instant>,
    /// Why the last frame scheduled another: (input, work, animating,
    /// egui-zero-delay). Surfaced in the 1 s frame log while diagnosing
    /// wake-loop bugs.
    last_wake_reason: (bool, bool, bool, bool),
    /// An explicit request for the next wake to render (scheduler, OS
    /// expose, input handlers). `about_to_wait` fires on every event-loop
    /// wake, Wayland frame callbacks after each present included, so
    /// rendering must be gated on intent or presenting itself keeps the
    /// loop hot forever.
    redraw_needed: bool,
    /// The render loop decided to sleep and painted one closing frame whose
    /// FPS reads "idle": a frozen number would look like a measurement.
    fps_display_idle: bool,
    /// Exponentially smoothed frame time (seconds). Updated every rendered
    /// frame, so the FPS display is live from the first measured interval
    /// after a wake, with no batching delay. `None` right after a sleep; the
    /// next frame's dt seeds it with a real measurement.
    smoothed_frame_s: Option<f32>,
    /// egui's repaint request from the last built frame.
    pending_ui_repaint: std::time::Duration,
    /// Reuse the last confirmed import options when opening the dialog again.
    last_step_import_detail: TessellationSettings,
    /// Pressed-mouse-button count; nonzero suppresses undo boundaries.
    mouse_buttons_down: u32,
    /// Which of the left, right and middle buttons are down, whoever took
    /// their presses: the camera lets go of a drag whose button is not.
    held_buttons: [bool; 3],
    /// The box a left drag from empty space in the view is drawing.
    box_select: Option<crate::app::box_select::BoxDrag>,
    /// Index-stable UUIDs for workbench overlay meshes (slot i -> pool[i]),
    /// so the renderer's per-body cache works for overlays too.
    overlay_id_pool: Vec<Uuid>,
    /// The regions passive features shade, meshed by the kernel, by
    /// feature with the revision they were meshed at.
    region_meshes: std::collections::HashMap<
        core_document::FeatureId,
        (u64, std::sync::Arc<kernel_api::TriMesh>),
    >,
    /// Latest keyboard modifiers from `WindowEvent::ModifiersChanged`.
    modifiers: winit::keyboard::ModifiersState,
    /// Stable renderer id for the face-highlight overlay slot.
    face_highlight_id: Uuid,
    /// Submission id of the hovered face's overlay.
    face_hover_id: Uuid,
    /// The overlay slot of the body a tree row under the pointer stands for.
    tree_hover_id: Uuid,
    /// The submission id of the whole-body selection overlay.
    body_highlight_id: Uuid,
    /// The submission ids of the hovered-edge and selected-edges outlines.
    edge_hover_id: Uuid,
    edge_select_id: Uuid,
    /// The print bed's submission id, and its line mesh keyed by the
    /// settings it was built from.
    print_bed_id: Uuid,
    print_bed: Option<(u64, std::sync::Arc<kernel_api::TriMesh>)>,
    origin_planes: app::scene_guides::OriginPlanes,
    /// The title the window currently shows; rewritten only on change.
    window_title: String,
    /// The thread the console and scripts run on; its engine keeps the
    /// console's globals between lines.
    script_thread: scripting::ScriptThread,
    /// Runs submitted and not finished, oldest (the running one) first.
    script_runs: std::collections::VecDeque<app::scripts::ScriptRun>,
    /// The running script's `doc.rebuild`, waiting on the kernel.
    script_rebuild: Option<app::scripts::RebuildWait>,
    /// A recording of what is done through the UI, while one is on.
    recording: Option<scripting::Recorder>,
    /// The files of the tabs closed, the latest last: what Reopen closed
    /// tab opens again.
    closed_files: Vec<PathBuf>,
    /// A picture of the view that arrived from the renderer, to be saved.
    picture: Option<render_wgpu::CapturedImage>,
    /// When the documents were last autosaved.
    autosaved_at: web_time::Instant,
    /// Each tab's document as last autosaved, by its mutation seq.
    autosaved: std::collections::HashMap<Uuid, u64>,
    /// Autosave writes on their threads, joined before an exit.
    autosave_jobs: Vec<platform::Job>,
    /// Copies a crash left, offered back on the start page.
    recoverable: Vec<app::recovery::Recoverable>,
    /// The unsaved-edits question on screen, and what waits for it.
    unsaved_asking: Option<app::unsaved::Asking>,
    /// A save over a file from another version, waiting for the user.
    save_over_asking: Option<app::file_safety::SaveOver>,
    /// Tabs whose edits the user answered for while quitting.
    quit_answered: std::collections::HashSet<Uuid>,
    /// The MCP server agents reach the document through.
    mcp: Option<app::mcp::McpServer>,
    /// Changes agents asked for, waiting for the user's OK.
    approvals: Vec<app::mcp::Approval>,
    /// An agent needs the user: the assistant panel opens.
    assistant_attention: bool,
    /// The chats with agents, in the order they were opened.
    chats: Vec<app::chats::Chat>,
    /// How many chats this session has opened, for the next one's name.
    chats_made: usize,
    /// Wakes the loop from another thread.
    waker: std::sync::Arc<dyn Fn() + Send + Sync>,
    /// Script files picked in a dialog, to run once it answers.
    scripts_to_run: Vec<PathBuf>,
    /// The scripts folder's scripts, and when it was last read.
    script_library: Vec<script_library::ScriptEntry>,
    /// The workbench packages found at start, and those installed, updated
    /// or removed while the app runs.
    packages: Vec<workbenches::PackageStatus>,
    /// Package installs, update checks and updates running on threads.
    package_work: app::packages::PackageWork,
    /// The workbench stores' indexes, and which are being fetched.
    store: app::packages::StoreView,
    /// What the last look for a newer printCAD found.
    release_check: app::updates::ReleaseCheck,
    script_library_read: Option<Instant>,
    /// A script printed or failed: the console opens to show it.
    console_attention: bool,
    /// Every command's id, read once the workbenches are registered.
    command_ids: Vec<String>,
    /// What the last run left: its open documents, window and panels.
    last_session: app::session_store::KeptSession,
    /// Files named on the command line, opened at start in place of the
    /// last session's.
    launch_files: Vec<PathBuf>,
    /// A browser page's window and renderer, set once the renderer, which
    /// the page may not wait for, is ready.
    #[cfg(target_arch = "wasm32")]
    web_gfx: std::rc::Rc<std::cell::RefCell<Option<(Renderer, winit::window::Window)>>>,
    #[cfg(target_arch = "wasm32")]
    proxy: winit::event_loop::EventLoopProxy<AppEvent>,
}

/// The bench a new document lands in. A registry with no non-modal bench
/// cannot host a document at all.
fn landing_workbench(registry: &DocumentService) -> WorkbenchId {
    registry
        .landing_workbench()
        .expect("a workbench that is not an edit session is registered")
}

/// The start page opens the session unless a bench hook asks for a
/// document or a scene straight away.
fn launch_screen() -> Screen {
    let straight_in = [
        "PRINTCAD_OPEN_FILE",
        "PRINTCAD_OPEN_DOC",
        "PRINTCAD_BENCH_SKETCH",
        "PRINTCAD_BENCH_ORBIT",
        "PRINTCAD_BENCH_SPIN",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some());
    if straight_in {
        Screen::Workspace
    } else {
        Screen::Start
    }
}

impl PrintCadApp {
    fn new(
        settings: RenderSettings,
        settings_store: SettingsStore,
        user_settings: UserSettings,
        registry: DocumentService,
        proxy: winit::event_loop::EventLoopProxy<AppEvent>,
    ) -> Self {
        let step_import_defaults = user_settings.import.tessellation.clone();
        let landing = ActiveWorkbench(landing_workbench(&registry));
        let session = app::session::DocumentSession::untitled(
            &user_settings.camera,
            landing,
            launch_screen(),
        );
        let tabs = vec![app::session::TabSlot {
            tab: session.tab,
            parked: None,
        }];
        Self {
            settings,
            session,
            tabs,
            active_tab: 0,
            import_owner: std::collections::HashMap::new(),
            frame_submission: FrameSubmission::default(),
            gfx: None,
            settings_store,
            user_settings,
            last_frame_time: None,
            current_fps: 0.0,
            gpu_name: None,
            graphics_api: None,
            available_gpus: Vec::new(),
            fps_accum_time: 0.0,
            fps_frame_count: 0,
            cursor_in_viewport: None,
            registry,
            file_dialog_rx: None,
            export_rx: None,
            last_export: Default::default(),
            kernel_worker: KernelWorker::spawn(),
            nav_device: app::sixdof::SixDofWorker::spawn({
                let proxy = proxy.clone();
                move || {
                    let _ = proxy.send_event(AppEvent::DeviceInput);
                }
            }),
            script_thread: {
                let proxy = std::sync::Mutex::new(proxy.clone());
                scripting::ScriptThread::spawn(move || {
                    if let Ok(proxy) = proxy.lock() {
                        let _ = proxy.send_event(AppEvent::Script);
                    }
                })
            },
            script_runs: Default::default(),
            script_rebuild: None,
            recording: None,
            closed_files: Vec::new(),
            picture: None,
            autosaved_at: web_time::Instant::now(),
            autosaved: std::collections::HashMap::new(),
            autosave_jobs: Vec::new(),
            recoverable: app::recovery::left_behind(),
            unsaved_asking: None,
            save_over_asking: None,
            quit_answered: Default::default(),
            mcp: None,
            approvals: Vec::new(),
            assistant_attention: false,
            chats: Vec::new(),
            chats_made: 0,
            waker: {
                let proxy = std::sync::Mutex::new(proxy.clone());
                std::sync::Arc::new(move || {
                    if let Ok(proxy) = proxy.lock() {
                        let _ = proxy.send_event(AppEvent::Agent);
                    }
                })
            },
            bench_open_fired: false,
            bench_select_fired: false,
            bench_picture_frames: 0,
            bench_task_fired: false,
            bench_workbench_fired: false,
            bench_repair_fired: false,
            bench_convert_fired: false,
            bench_refine_fired: false,
            bench_click_frames: 0,
            bench_started: Instant::now(),
            frame_phase_accum: (0.0, 0.0, 0),
            scene_redraw_accum: 0,
            scene_redraws_per_s: 0,
            status_changes_accum: 0,
            last_status_text: None,
            last_input_time: None,
            last_wake_reason: (false, false, false, false),
            redraw_needed: true,
            scripts_to_run: Vec::new(),
            script_library: Vec::new(),
            packages: Vec::new(),
            package_work: Default::default(),
            store: Default::default(),
            release_check: Default::default(),
            script_library_read: None,
            console_attention: false,
            command_ids: Vec::new(),
            last_session: app::session_store::last_session(),
            launch_files: Vec::new(),
            #[cfg(target_arch = "wasm32")]
            web_gfx: Default::default(),
            #[cfg(target_arch = "wasm32")]
            proxy: proxy.clone(),
            fps_display_idle: false,
            smoothed_frame_s: None,
            pending_ui_repaint: std::time::Duration::MAX,
            last_step_import_detail: step_import_defaults,
            mouse_buttons_down: 0,
            held_buttons: [false; 3],
            box_select: None,
            overlay_id_pool: Vec::new(),
            region_meshes: Default::default(),
            modifiers: winit::keyboard::ModifiersState::default(),
            face_highlight_id: Uuid::new_v4(),
            face_hover_id: Uuid::new_v4(),
            tree_hover_id: Uuid::new_v4(),
            body_highlight_id: Uuid::new_v4(),
            edge_hover_id: Uuid::new_v4(),
            edge_select_id: Uuid::new_v4(),
            print_bed_id: Uuid::new_v4(),
            print_bed: None,
            origin_planes: app::scene_guides::OriginPlanes::new(),
            window_title: String::new(),
            recent: app::doc_io::load_recent(),
        }
    }

    /// The bench a new document lands in, as the registry orders them.
    pub(crate) fn landing_workbench(&self) -> ActiveWorkbench {
        ActiveWorkbench(landing_workbench(&self.registry))
    }

    fn active_workbench_id(&self) -> WorkbenchId {
        self.session.active_workbench.0.clone()
    }

    /// Call on_deactivate on a workbench. It runs inside a switch, so its
    /// requests apply except another switch.
    fn call_workbench_deactivate(&mut self, wb_id: &WorkbenchId) {
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(wb_id, params, |wb, ctx| wb.on_deactivate(ctx))
        {
            self.apply_hook_outcome(outcome, app::workbench_host::HookSite::Lifecycle);
        }
    }

    /// Call on_activate on a workbench. It runs inside a switch, so its
    /// requests apply except another switch.
    fn call_workbench_activate(&mut self, wb_id: &WorkbenchId) {
        let params = self.interaction_ctx_params();
        if let Some(((), outcome)) =
            self.with_workbench_ctx(wb_id, params, |wb, ctx| wb.on_activate(ctx))
        {
            self.apply_hook_outcome(outcome, app::workbench_host::HookSite::Lifecycle);
        }
    }
}

impl Drop for PrintCadApp {
    fn drop(&mut self) {
        // Teardown timing: each big owner logs its own phase; this marks
        // the start so the gaps between them are attributable.
        tracing::info!(target: "printcad.frame", "app teardown begins");
    }
}

impl ApplicationHandler<AppEvent> for PrintCadApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.init_gfx(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.handle_window_event(event_loop, window_id, event);
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            // The device thread only knocks when the puck starts or stops
            // moving, or a button changes; while it is deflected the frame
            // loop keeps itself awake.
            AppEvent::DeviceInput | AppEvent::Script | AppEvent::Agent => {
                self.redraw_needed = true;
            }
            #[cfg(target_arch = "wasm32")]
            AppEvent::GfxReady => {
                let ready = self.web_gfx.borrow_mut().take();
                if let Some((renderer, window)) = ready {
                    self.finish_gfx(renderer, window);
                    self.redraw_needed = true;
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.frame(event_loop);
    }
}

/// The name the desktop entry and the window share, so a Linux desktop
/// finds the window's icon.
#[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
const APP_ID: &str = "printcad";

/// The application's icon, for the window on the systems that draw one
/// (Windows, X11); macOS takes it from the bundle.
#[cfg(not(target_arch = "wasm32"))]
fn window_icon() -> Option<winit::window::Icon> {
    let bytes: &[u8] = include_bytes!("../assets/icon/printcad-256.png");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut rgba = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut rgba).ok()?;
    rgba.truncate(frame.buffer_size());
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    winit::window::Icon::from_rgba(rgba, frame.width, frame.height).ok()
}

impl PrintCadApp {
    /// Create the window, renderer, and UI layer once the event loop is live.
    #[cfg(not(target_arch = "wasm32"))]
    fn init_gfx(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("printCAD".to_string())
            .with_window_icon(window_icon());
        let attributes =
            app::session_store::placed_window(attributes, self.last_session.window, event_loop);
        // Wayland shows the icon of the desktop entry the app id names.
        #[cfg(all(unix, not(target_os = "macos"), not(target_os = "android")))]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, APP_ID, APP_ID,
        );
        let window = match event_loop.create_window(attributes) {
            Ok(window) => window,
            Err(err) => {
                error!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };

        let mut renderer = Renderer::new(self.settings.clone());
        if let Err(err) = renderer.initialize(&window, event_loop.owned_display_handle()) {
            error!("failed to initialize renderer: {err}");
            event_loop.exit();
            return;
        }
        self.finish_gfx(renderer, window);
    }

    /// A browser page: a canvas on the page, and the renderer made on it
    /// without waiting, since a page may not block. [`AppEvent::GfxReady`]
    /// says when it is there.
    #[cfg(target_arch = "wasm32")]
    fn init_gfx(&mut self, event_loop: &ActiveEventLoop) {
        use winit::platform::web::WindowAttributesExtWebSys;
        if self.gfx.is_some() || self.web_gfx.borrow().is_some() {
            return;
        }
        // The page's size at first; the canvas follows the page from then on.
        let page = web_sys::window().map(|w| {
            let px = |v: Result<wasm_bindgen::JsValue, _>| v.ok().and_then(|v| v.as_f64());
            (
                px(w.inner_width()).unwrap_or(1280.0),
                px(w.inner_height()).unwrap_or(800.0),
            )
        });
        let (width, height) = page.unwrap_or((1280.0, 800.0));
        let attributes = WindowAttributes::default()
            .with_title("printCAD".to_string())
            .with_inner_size(winit::dpi::LogicalSize::new(width, height))
            .with_append(true);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => window,
            Err(err) => {
                error!("failed to create the page's canvas: {err}");
                return;
            }
        };
        let mut renderer = Renderer::new(self.settings.clone());
        let display = event_loop.owned_display_handle();
        let (slot, proxy) = (self.web_gfx.clone(), self.proxy.clone());
        wasm_bindgen_futures::spawn_local(async move {
            match renderer.initialize_async(&window, display).await {
                Ok(()) => {
                    *slot.borrow_mut() = Some((renderer, window));
                    let _ = proxy.send_event(AppEvent::GfxReady);
                }
                Err(err) => error!("failed to initialize renderer: {err}"),
            }
        });
    }

    /// The window and its renderer are made: the UI on them, and the
    /// camera sized to the window.
    fn finish_gfx(&mut self, renderer: Renderer, window: winit::window::Window) {
        // A page's canvas was sized while its renderer was being made, and
        // the resize came before there was a renderer to hear it.
        #[cfg(target_arch = "wasm32")]
        let renderer = {
            let mut renderer = renderer;
            render_wgpu::RenderBackend::resize(&mut renderer, window.inner_size());
            renderer
        };
        let mut ui_layer = UiLayer::new(&window);
        if let Some(panels) = &self.last_session.panels {
            ui_layer.restore_panel_layout(panels);
        }
        self.gpu_name = renderer.gpu_name().map(|s| s.to_string());
        self.graphics_api = renderer.graphics_api();
        if let Some(list) = renderer.available_gpus() {
            self.available_gpus = list.to_vec();
        }
        let size = window.inner_size();
        self.session
            .camera
            .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
        let window_id = window.id();
        self.gfx = Some(app::Gfx {
            renderer,
            ui_layer,
            window,
            window_id,
        });
    }
}

#[cfg(test)]
mod icon_tests {
    #[test]
    fn the_window_icon_decodes_from_the_bundled_picture() {
        assert!(super::window_icon().is_some());
    }
}
