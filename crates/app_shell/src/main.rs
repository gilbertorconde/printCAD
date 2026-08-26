mod app;
mod camera;
mod kernel_worker;
mod log_panel;
mod orientation_cube;
mod ui;

use anyhow::{Context, Result};
use app::doc_io::FileDialogResult;
use camera::CameraController;
use core_document::{BodyId, Document, DocumentService, WorkbenchId};
use kernel_api::TessellationSettings;
use kernel_worker::KernelWorker;
use log_panel as app_log;
use render_vk::{FrameSubmission, RenderBackend, RenderSettings, VulkanRenderer};
use settings::{SettingsStore, UserSettings};
use std::path::PathBuf;

use std::time::Instant;
use tracing::error;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use ui::{ActiveTool, ActiveWorkbench, TreeItemId, UiLayer};
use uuid::Uuid;
use winit::{
    application::ApplicationHandler,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    window::{WindowAttributes, WindowId},
};
use workbenches::register_all_workbenches;

fn init_tracing_subscriber() -> anyhow::Result<Option<tracing_appender::non_blocking::WorkerGuard>>
{
    let stdout_filter =
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
            .with(fmt::layer().with_filter(stdout_filter.clone()))
            .with(file_layer)
            .init();

        Ok(Some(guard))
    } else {
        tracing_subscriber::registry()
            .with(fmt::layer().with_filter(stdout_filter))
            .init();

        Ok(None)
    }
}

fn main() -> Result<()> {
    let _camera_tracing_guard =
        init_tracing_subscriber().context("tracing subscriber init failed")?;

    let document = Document::new("Untitled");
    let mut registry = DocumentService::default();
    register_all_workbenches(&mut registry)?;

    app_log::info(format!(
        "Registered {} workbenches",
        registry.workbench_descriptors().count()
    ));
    app_log::info(format!(
        "Loaded document `{}` ({})",
        document.name(),
        document.id()
    ));

    let settings_store = SettingsStore::new().context("settings store init failed")?;
    let user_settings = match settings_store.load() {
        Ok(settings) => settings,
        Err(err) => {
            app_log::warn(format!("Using default settings (failed to load): {err}"));
            UserSettings::default()
        }
    };

    let event_loop = EventLoop::new().context("failed to create event loop")?;
    let render_settings = RenderSettings {
        preferred_gpu: user_settings.preferred_gpu.clone(),
        msaa_samples: user_settings.rendering.msaa_samples,
        ..RenderSettings::default()
    };
    let mut app = PrintCadApp::new(
        render_settings,
        settings_store,
        user_settings,
        document,
        registry,
    );
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

struct PrintCadApp {
    settings: RenderSettings,
    frame_submission: FrameSubmission,
    /// Window + renderer + UI layer; teardown order is enforced by the
    /// [`app::Gfx`] struct's field order (renderer before window).
    gfx: Option<app::Gfx>,
    settings_store: SettingsStore,
    user_settings: UserSettings,
    camera: CameraController,
    active_tool: ActiveTool,
    last_frame_time: Option<Instant>,
    current_fps: f32,
    gpu_name: Option<String>,
    available_gpus: Vec<String>,
    fps_accum_time: f32,
    fps_frame_count: u32,
    // Selected body ID (for highlighting/selection)
    selected_body: Option<Uuid>,
    // Hovered body ID (for highlighting)
    hovered_body: Option<Uuid>,
    // Hovered world position (for status bar display)
    hovered_world_pos: Option<[f32; 3]>,
    // Current cursor position in viewport
    cursor_in_viewport: Option<(f32, f32)>,
    // Document and workbench registry
    document: Document,
    registry: DocumentService,
    // Currently active workbench (determines which tools are visible)
    active_workbench: ActiveWorkbench,
    // Active document object (selected feature in tree - separate from editing mode)
    active_document_object: Option<core_document::FeatureId>,
    active_body_id: Option<BodyId>,
    tree_selection: Option<TreeItemId>,
    // Current file on disk (if any).
    current_file: Option<PathBuf>,
    // Pending file dialog result from background thread.
    file_dialog_rx: Option<std::sync::mpsc::Receiver<FileDialogResult>>,
    // Background worker that owns the geometry kernel. STEP imports run there
    // so the viewport stays interactive while a multi-million-tri model is
    // tessellated; responses are drained once per frame in `about_to_wait`.
    kernel_worker: KernelWorker,
    /// The document server connection — local daemon by default, direct
    /// files when no daemon can run, a remote plugin someday. Everything
    /// that crosses it is the wire protocol; `document_load_epoch` rides
    /// Open requests as the token that invalidates late responses.
    server: Box<dyn core_document::server::DocumentServer>,
    /// The socket the server connection is (or should be) on — per-document
    /// once the document has a file, per-session for Untitled. Reconnects
    /// and connection switches aim here.
    server_socket: PathBuf,
    /// Last reconnect attempt, so a dead daemon is retried at a gentle pace
    /// instead of every frame.
    last_server_reconnect: Option<Instant>,
    /// Dev/bench hook: `PRINTCAD_OPEN_FILE` triggers one STEP import at
    /// startup, so a benchmark run needs no dialog interaction.
    bench_open_fired: bool,
    /// Rolling per-phase frame cost, emitted once a second alongside the FPS
    /// counter (target `printcad.frame`): (ui ms, render ms, frames).
    frame_phase_accum: (f32, f32, u32),
    /// Last frame's view-projection, to detect camera motion. While the
    /// camera moves the edge pass is skipped (see `FrameSubmission::
    /// suppress_edges`); the first still frame restores it.
    prev_view_proj: Option<[[f32; 4]; 4]>,
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
    /// wake — including Wayland frame callbacks after each present — so
    /// rendering must be gated on intent or presenting itself keeps the
    /// loop hot forever.
    redraw_needed: bool,
    /// The render loop decided to sleep and painted one closing frame whose
    /// FPS reads "idle" — a frozen number would look like a measurement.
    fps_display_idle: bool,
    /// Exponentially smoothed frame time (seconds). Updated every rendered
    /// frame, so the FPS display is live from the first measured interval
    /// after a wake — no batching delay. `None` right after a sleep; the
    /// next frame's dt seeds it with a real measurement.
    smoothed_frame_s: Option<f32>,
    /// egui's repaint request from the last built frame.
    pending_ui_repaint: std::time::Duration,
    document_load_epoch: u64,
    /// Picked STEP path and draft tessellation settings until the user confirms import.
    step_import_pending: Option<(PathBuf, TessellationSettings)>,
    /// Reuse the last confirmed import options when opening the dialog again.
    last_step_import_detail: TessellationSettings,
    /// Snapshot-based undo/redo. `note()` is called once per frame while no
    /// mouse button is held, so drags coalesce into single steps.
    undo: core_document::undo::UndoHistory,
    /// Pressed-mouse-button count; nonzero suppresses undo boundaries.
    mouse_buttons_down: u32,
    /// Index-stable UUIDs for workbench overlay meshes (slot i -> pool[i]),
    /// so the renderer's per-body cache works for overlays too.
    overlay_id_pool: Vec<Uuid>,
    /// Latest keyboard modifiers from `WindowEvent::ModifiersChanged`.
    modifiers: winit::keyboard::ModifiersState,
    /// A workbench asked to create a sketch on this body; carried between
    /// hooks until the sketch workbench consumes it (plane picker).
    pending_sketch_creation: Option<core_document::SketchAttachRequest>,
    /// Face under the most recent body selection click (surface point +
    /// normal derived from the picked mesh triangle).
    last_face_hit: Option<(Uuid, core_document::FaceRef)>,
    /// Extracted coplanar sub-mesh of the selected face, rendered as a
    /// highlight overlay. Present only while a face (not the whole body)
    /// is the selection.
    face_highlight: Option<app::input::FaceHighlight>,
    /// Sketch feature under the cursor (CPU hit-test, drives hover tint).
    hovered_sketch: Option<core_document::FeatureId>,
    /// Stable renderer id for the face-highlight overlay slot.
    face_highlight_id: Uuid,
    /// Timestamp + target of the last selection click (double-click detect).
    last_select_click: Option<(Instant, Uuid)>,
    /// Workbench to return to when sketch editing finishes, when the sketch
    /// flow was started from another workbench (e.g. Part Design).
    return_workbench: Option<ActiveWorkbench>,
}

impl PrintCadApp {
    fn new(
        settings: RenderSettings,
        settings_store: SettingsStore,
        user_settings: UserSettings,
        document: Document,
        registry: DocumentService,
    ) -> Self {
        let camera = CameraController::new(&user_settings.camera, (1, 1));
        let undo = core_document::undo::UndoHistory::new(&document, 64);

        // The document server: a per-session local daemon by default; plain
        // in-process file I/O when the daemon cannot start. Same contract
        // either way — the trait is the seam a remote plugin replaces.
        let server_socket = doc_server::socket_path_for_untitled();
        let server: Box<dyn core_document::server::DocumentServer> =
            match doc_server::DaemonClient::spawn_or_connect(&server_socket) {
                Ok(client) => Box::new(client),
                Err(err) => {
                    tracing::warn!("document daemon unavailable ({err}); using direct file I/O");
                    Box::new(doc_server::DirectFiles::new())
                }
            };
        tracing::info!(server = server.name(), "document server connected");

        Self {
            settings,
            frame_submission: FrameSubmission::default(),
            gfx: None,
            settings_store,
            user_settings,
            camera,
            active_tool: ActiveTool::default(),
            last_frame_time: None,
            current_fps: 0.0,
            gpu_name: None,
            available_gpus: Vec::new(),
            fps_accum_time: 0.0,
            fps_frame_count: 0,
            selected_body: None,
            hovered_body: None,
            hovered_world_pos: None,
            cursor_in_viewport: None,
            document,
            registry,
            active_workbench: ActiveWorkbench::default(),
            active_document_object: None,
            active_body_id: None,
            tree_selection: Some(TreeItemId::DocumentRoot),
            current_file: None,
            file_dialog_rx: None,
            kernel_worker: KernelWorker::spawn(),
            server,
            server_socket,
            last_server_reconnect: None,
            document_load_epoch: 0,
            bench_open_fired: false,
            frame_phase_accum: (0.0, 0.0, 0),
            prev_view_proj: None,
            last_input_time: None,
            last_wake_reason: (false, false, false, false),
            redraw_needed: true,
            fps_display_idle: false,
            smoothed_frame_s: None,
            pending_ui_repaint: std::time::Duration::MAX,
            step_import_pending: None,
            last_step_import_detail: TessellationSettings::default(),
            undo,
            mouse_buttons_down: 0,
            overlay_id_pool: Vec::new(),
            modifiers: winit::keyboard::ModifiersState::default(),
            pending_sketch_creation: None,
            last_face_hit: None,
            face_highlight: None,
            hovered_sketch: None,
            face_highlight_id: Uuid::new_v4(),
            last_select_click: None,
            return_workbench: None,
        }
    }

    /// Get the workbench ID for the currently active workbench.
    fn active_workbench_id(&self) -> WorkbenchId {
        self.active_workbench.0.clone()
    }

    /// Call on_deactivate on a workbench.
    fn call_workbench_deactivate(&mut self, wb_id: &WorkbenchId) {
        let params = self.interaction_ctx_params();
        self.with_workbench_ctx(wb_id, params, |wb, ctx| wb.on_deactivate(ctx));
    }

    /// Call on_activate on a workbench.
    fn call_workbench_activate(&mut self, wb_id: &WorkbenchId) {
        let params = self.interaction_ctx_params();
        self.with_workbench_ctx(wb_id, params, |wb, ctx| wb.on_activate(ctx));
    }
}

impl ApplicationHandler for PrintCadApp {
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.frame(event_loop);
    }
}

impl PrintCadApp {
    /// Create the window, renderer, and UI layer once the event loop is live.
    fn init_gfx(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }

        let window = match event_loop.create_window(
            WindowAttributes::default().with_title("printCAD (prototype)".to_string()),
        ) {
            Ok(window) => window,
            Err(err) => {
                error!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };

        let mut renderer = VulkanRenderer::new(self.settings.clone());
        if let Err(err) = renderer.initialize(&window) {
            error!("failed to initialize renderer: {err}");
            event_loop.exit();
            return;
        }

        let ui_layer = UiLayer::new(&window);
        self.gpu_name = renderer.gpu_name().map(|s| s.to_string());
        if let Some(list) = renderer.available_gpus() {
            self.available_gpus = list.to_vec();
        }
        let size = window.inner_size();
        self.camera
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
