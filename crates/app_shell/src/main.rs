// A release build on Windows is a windowed program: no console opens beside
// it. `--mcp` still speaks over the pipes its agent hands it.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod camera;
mod console;
mod headless;
mod kernel_worker;
mod log_panel;
mod orientation_cube;
mod platform;
mod script_library;
#[cfg(test)]
mod test_support;
mod ui;

use anyhow::{Context, Result};
use core_document::{Document, DocumentService, WorkbenchId};
use kernel_worker::KernelWorker;
use log_panel as app_log;
use render_wgpu::RenderBackend;
use render_wgpu::{FrameSubmission, RenderSettings, Renderer};
use settings::{SettingsStore, UserSettings};
use std::path::PathBuf;

use tracing::error;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};
use ui::{ActiveWorkbench, Screen, UiLayer};
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{WindowAttributes, WindowId},
};
use workbenches::register_all_workbenches;

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

fn main() -> Result<()> {
    core_document::set_writer_version(env!("CARGO_PKG_VERSION"));
    app::bench_hooks::hooks();
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

    // From here a panic leaves a report a developer can replay.
    app::crash::install();
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
    app.packages.installed = packages;
    // `printcad <file>…`, as a desktop entry or file manager starts it.
    app.kept.launch_files = words
        .iter()
        .filter(|word| !word.starts_with('-'))
        .map(PathBuf::from)
        .collect();
    // `PRINTCAD_NO_UPDATE_CHECKS` leaves out the network looks a start
    // makes on its own (packages' updates, the stores' lists, the app's
    // latest release), so many runs at once stay within GitHub's limits;
    // asked for from Preferences or the Help menu, they still go.
    let looks_at_start = std::env::var_os("PRINTCAD_NO_UPDATE_CHECKS").is_none();
    if looks_at_start && user_settings_check_updates {
        app.check_package_updates();
        // The stores' lists, for whoever has a package one took off.
        app.look_at_stores(true);
    }
    if looks_at_start && app.user_settings.updates.check_at_start {
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
            store.disable_saving();
            app_log::warn(format!(
                "Using default settings; saving is disabled (failed to load): {err}"
            ));
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
    /// A `with_tab` turn has a tab other than the one on screen in
    /// `session`, while the benches hold the on-screen tab's state.
    visiting: bool,
    imports: app::app_state::Imports,
    frame_submission: FrameSubmission,
    /// Window + renderer + UI layer; teardown order is enforced by the
    /// [`app::Gfx`] struct's field order (renderer before window).
    gfx: Option<app::Gfx>,
    settings_store: SettingsStore,
    user_settings: UserSettings,
    stats: app::app_state::FrameStats,
    gpu: app::app_state::GpuInfo,
    input: app::app_state::InputState,
    registry: DocumentService,
    files: app::app_state::FileWork,
    // Background worker that owns the geometry kernel. STEP imports run there
    // so the viewport stays interactive while a multi-million-tri model is
    // tessellated; responses are drained once per frame in `about_to_wait`.
    kernel_worker: KernelWorker,
    /// Background reader for a 6-DoF mouse. It holds the puck's current
    /// deflection; the frame loop integrates it.
    nav_device: app::sixdof::SixDofWorker,
    /// How far the run's bench hooks (`app::bench_hooks`) have got.
    bench: app::bench_hooks::BenchProgress,
    /// An explicit request for the next wake to render (scheduler, OS
    /// expose, input handlers). `about_to_wait` fires on every event-loop
    /// wake, Wayland frame callbacks after each present included, so
    /// rendering must be gated on intent or presenting itself keeps the
    /// loop hot forever.
    redraw_needed: bool,
    /// egui's repaint request from the last built frame.
    pending_ui_repaint: std::time::Duration,
    /// The tools active last frame, to count each one started.
    tools_last_frame: Vec<String>,
    overlay_ids: app::app_state::OverlayIds,
    scene_meshes: app::app_state::SceneMeshes,
    /// The title the window currently shows; rewritten only on change.
    window_title: String,
    scripting: app::app_state::Scripting,
    autosave: app::app_state::Autosave,
    questions: app::app_state::Questions,
    agents: app::app_state::Agents,
    packages: app::app_state::Packages,
    /// Every command's id, read once the workbenches are registered.
    command_ids: Vec<String>,
    kept: app::app_state::KeptRun,
}

/// The bench a new document lands in. A registry with no non-modal bench
/// cannot host a document at all.
fn landing_workbench(registry: &DocumentService) -> WorkbenchId {
    registry
        .landing_workbench()
        .expect("a workbench that is not an edit session is registered")
}

/// The start page opens the session unless a bench hook is set: a hook's
/// run goes straight to a document or a scene.
fn launch_screen() -> Screen {
    if app::bench_hooks::hooks().any() {
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
            visiting: false,
            imports: app::app_state::Imports::new(step_import_defaults),
            frame_submission: FrameSubmission::default(),
            gfx: None,
            settings_store,
            user_settings,
            stats: Default::default(),
            gpu: Default::default(),
            input: Default::default(),
            registry,
            files: app::app_state::FileWork::new(app::doc_io::load_recent()),
            kernel_worker: KernelWorker::spawn(),
            nav_device: app::sixdof::SixDofWorker::spawn({
                let proxy = proxy.clone();
                move || {
                    let _ = proxy.send_event(AppEvent::DeviceInput);
                }
            }),
            bench: Default::default(),
            redraw_needed: true,
            pending_ui_repaint: std::time::Duration::MAX,
            tools_last_frame: Vec::new(),
            overlay_ids: Default::default(),
            scene_meshes: Default::default(),
            window_title: String::new(),
            scripting: app::app_state::Scripting::new({
                let proxy = std::sync::Mutex::new(proxy.clone());
                scripting::ScriptThread::spawn(move || {
                    if let Ok(proxy) = proxy.lock() {
                        let _ = proxy.send_event(AppEvent::Script);
                    }
                })
            }),
            autosave: app::app_state::Autosave::new(app::recovery::left_behind()),
            questions: Default::default(),
            agents: app::app_state::Agents::new({
                let proxy = std::sync::Mutex::new(proxy.clone());
                std::sync::Arc::new(move || {
                    if let Ok(proxy) = proxy.lock() {
                        let _ = proxy.send_event(AppEvent::Agent);
                    }
                })
            }),
            packages: Default::default(),
            command_ids: Vec::new(),
            kept: app::app_state::KeptRun::new(app::session_store::last_session()),
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
    fn init_gfx(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("printCAD".to_string())
            .with_window_icon(window_icon());
        let attributes = app::session_store::placed_window(
            attributes,
            self.kept.last_session.window,
            event_loop,
        );
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

    /// The window and its renderer are made: the UI on them, and the
    /// camera sized to the window.
    fn finish_gfx(&mut self, renderer: Renderer, window: winit::window::Window) {
        let mut ui_layer = UiLayer::new(&window);
        if let Some(panels) = &self.kept.last_session.panels {
            ui_layer.restore_panel_layout(panels);
        }
        self.gpu.name = renderer.gpu_name().map(|s| s.to_string());
        self.gpu.api = renderer.graphics_api();
        if let Some(list) = renderer.available_gpus() {
            self.gpu.available = list.to_vec();
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
