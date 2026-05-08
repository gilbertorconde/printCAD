mod camera;
mod kernel_worker;
mod log_panel;
mod orientation_cube;
mod ui;

use anyhow::{Context, Result};
use camera::{CameraController, CameraPointerResult};
use core_document::{
    BodyId, Document, DocumentService, ImportedGeometry, LogLevel,
    MouseButton as WbMouseButton, Unit, WorkbenchFeature, WorkbenchId, WorkbenchInputEvent,
    WorkbenchRuntimeContext,
};
use glam::{Vec2, Vec3};
use kernel_api::{ImportedModel, LengthUnit, TessellationSettings};
use kernel_worker::{KernelResponse, KernelWorker};
use log_panel as app_log;
use orientation_cube::OrientationCubeInput;
use render_vk::{
    BodySubmission, FrameSubmission, GpuLight, HighlightState, LightingData, RenderBackend,
    RenderSettings, ViewportRect as RenderViewportRect, VulkanRenderer,
};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};
use settings::{LightingSettings, SettingsStore, UserSettings};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{error, info};
use tracing_subscriber::{
    prelude::*,
    fmt,
    EnvFilter,
};
use ui::{ActiveTool, ActiveWorkbench, TreeItemId, UiLayer};
use uuid::Uuid;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};
use workbenches::register_all_workbenches;

/// Map a STEP-declared length unit onto the document's display unit enum.
fn length_unit_to_document_unit(unit: LengthUnit) -> Unit {
    match unit {
        LengthUnit::Millimetre => Unit::Mm,
        LengthUnit::Centimetre => Unit::Cm,
        LengthUnit::Metre => Unit::M,
        LengthUnit::Inch => Unit::In,
        LengthUnit::Foot => Unit::Ft,
    }
}

/// Stable u64 fingerprint of a serde JSON value. Used as a `revision`
/// counter for sketch geometry so the GPU mesh cache can skip the upload
/// when the underlying sketch JSON hasn't changed between frames.
fn hash_revision(value: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Union of all imported mesh AABBs in world space `(min, max)`.
fn document_imported_aabb(document: &Document) -> Option<(Vec3, Vec3)> {
    let mut combined_min = [f32::INFINITY; 3];
    let mut combined_max = [f32::NEG_INFINITY; 3];
    let mut any = false;
    for (_, g) in document.imported_geometries() {
        if let Some((min, max)) = g.mesh.bounds() {
            any = true;
            for axis in 0..3 {
                combined_min[axis] = combined_min[axis].min(min[axis]);
                combined_max[axis] = combined_max[axis].max(max[axis]);
            }
        }
    }
    if !any || combined_min[0] > combined_max[0] {
        return None;
    }
    Some((
        Vec3::new(combined_min[0], combined_min[1], combined_min[2]),
        Vec3::new(combined_max[0], combined_max[1], combined_max[2]),
    ))
}

fn aabb_fit_center_radius(aabb_min: Vec3, aabb_max: Vec3) -> (Vec3, f32) {
    let center = (aabb_min + aabb_max) * 0.5;
    let extents = aabb_max - aabb_min;
    let radius = extents.length() * 0.5;
    (center, radius.max(1.0))
}

fn init_tracing_subscriber(
) -> anyhow::Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
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
    let _camera_tracing_guard = init_tracing_subscriber().context("tracing subscriber init failed")?;

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
    let mut render_settings = RenderSettings::default();
    render_settings.preferred_gpu = user_settings.preferred_gpu.clone();
    render_settings.msaa_samples = user_settings.rendering.msaa_samples;
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

/// Button labels for the unsaved-changes dialog. GTK and Zenity backends report
/// `MessageDialogResult::Custom(label)` for `YesNoCancelCustom`, not Yes/No/Cancel.
const UNSAVED_CHANGES_SAVE: &str = "Save";
const UNSAVED_CHANGES_DISCARD: &str = "Discard";
const UNSAVED_CHANGES_CANCEL: &str = "Cancel";

struct PrintCadApp {
    settings: RenderSettings,
    /// Declared before GPU/window so it drops last — not used during teardown.
    frame_submission: FrameSubmission,
    // `Window` must outlive `VulkanRenderer` (surface/swapchain). Rust drops
    // fields in reverse declaration order, so `renderer` must appear *after*
    // `window`/`ui_layer` so GPU teardown runs while the native window exists.
    window: Option<Window>,
    window_id: Option<WindowId>,
    ui_layer: Option<UiLayer>,
    renderer: Option<VulkanRenderer>,
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
    // Background worker that owns the OCCT kernel. STEP imports run there
    // so the viewport stays interactive while a multi-million-tri model is
    // tessellated; responses are drained once per frame in `about_to_wait`.
    kernel_worker: KernelWorker,
}

enum FileDialogKind {
    Open,
    Save,
    SaveAs,
    ImportStep,
}

struct FileDialogResult {
    kind: FileDialogKind,
    path: Option<PathBuf>,
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

        Self {
            settings,
            frame_submission: FrameSubmission::default(),
            window: None,
            window_id: None,
            ui_layer: None,
            renderer: None,
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
        }
    }

    /// Get the workbench ID for the currently active workbench.
    fn active_workbench_id(&self) -> WorkbenchId {
        self.active_workbench.0.clone()
    }

    /// Flush log entries to the app log panel.
    fn flush_logs(logs: Vec<core_document::LogEntry>) {
        for entry in logs {
            match entry.level {
                LogLevel::Info => app_log::info(entry.message),
                LogLevel::Warn => app_log::warn(entry.message),
                LogLevel::Error => app_log::error(entry.message),
            }
        }
    }

    /// Call on_deactivate on a workbench.
    fn call_workbench_deactivate(&mut self, wb_id: &WorkbenchId) {
        // Collect camera/viewport info first
        let cam_pos = self.camera.position();
        let cam_target = self.camera.target();
        let vp = self.camera.viewport_info();
        let hovered_world_pos = self.hovered_world_pos;
        let hovered_body_id = self.hovered_body;
        let selected_body_id = self.selected_body;
        let cursor_viewport_pos = self.cursor_in_viewport;

        // Get workbench and call hook
        if let Ok(wb) = self.registry.workbench_mut(wb_id) {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut self.document,
                cam_pos,
                cam_target,
                (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            );
            ctx.hovered_world_pos = hovered_world_pos;
            ctx.hovered_body_id = hovered_body_id;
            ctx.selected_body_id = selected_body_id;
            ctx.cursor_viewport_pos = cursor_viewport_pos;

            wb.on_deactivate(&mut ctx);
            Self::flush_logs(ctx.drain_logs());
        }
    }

    /// Call on_activate on a workbench.
    fn call_workbench_activate(&mut self, wb_id: &WorkbenchId) {
        // Collect camera/viewport info first
        let cam_pos = self.camera.position();
        let cam_target = self.camera.target();
        let vp = self.camera.viewport_info();
        let hovered_world_pos = self.hovered_world_pos;
        let hovered_body_id = self.hovered_body;
        let selected_body_id = self.selected_body;
        let cursor_viewport_pos = self.cursor_in_viewport;

        // Get workbench and call hook
        if let Ok(wb) = self.registry.workbench_mut(wb_id) {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut self.document,
                cam_pos,
                cam_target,
                (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            );
            ctx.hovered_world_pos = hovered_world_pos;
            ctx.hovered_body_id = hovered_body_id;
            ctx.selected_body_id = selected_body_id;
            ctx.cursor_viewport_pos = cursor_viewport_pos;

            wb.on_activate(&mut ctx);
            Self::flush_logs(ctx.drain_logs());
        }
    }

    fn document_has_asset_files(&self) -> bool {
        self.document.assets().next().is_some()
    }

    /// If the document is dirty, prompt Save / Discard / Cancel. Returns false when
    /// the user cancels or save fails.
    fn confirm_discard_or_save(&mut self) -> bool {
        if !self.document.metadata().dirty() {
            return true;
        }
        let res = MessageDialog::new()
            .set_title("Unsaved changes")
            .set_description(
                "Save changes before continuing? Save writes the file, Discard loses edits, Cancel stays here.",
            )
            .set_level(MessageLevel::Warning)
            .set_buttons(MessageButtons::YesNoCancelCustom(
                UNSAVED_CHANGES_SAVE.into(),
                UNSAVED_CHANGES_DISCARD.into(),
                UNSAVED_CHANGES_CANCEL.into(),
            ))
            .show();
        match res {
            MessageDialogResult::Cancel => false,
            MessageDialogResult::No => true,
            MessageDialogResult::Yes => self.save_document_interactive(),
            MessageDialogResult::Custom(s) if s == UNSAVED_CHANGES_SAVE => {
                self.save_document_interactive()
            }
            MessageDialogResult::Custom(s) if s == UNSAVED_CHANGES_DISCARD => true,
            MessageDialogResult::Custom(s) if s == UNSAVED_CHANGES_CANCEL => false,
            _ => false,
        }
    }

    /// Save to [`Self::current_file`] or prompt for a path. Returns false if cancelled or save fails.
    fn save_document_interactive(&mut self) -> bool {
        let path = if let Some(ref p) = self.current_file {
            p.clone()
        } else {
            let mut dialog = FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]);
            if let Ok(recent_path) = SettingsStore::recent_file_path() {
                if let Ok(file) = std::fs::File::open(&recent_path) {
                    if let Ok(saved_dir_str) = serde_json::from_reader::<_, String>(file) {
                        dialog = dialog.set_directory(std::path::PathBuf::from(saved_dir_str));
                    }
                }
            }
            match dialog.set_file_name("untitled.prtcad").save_file() {
                Some(p) => p,
                None => return false,
            }
        };
        match self.save_document_at(&path) {
            Ok(()) => true,
            Err(err) => {
                app_log::error(format!("Save failed: {err:#}"));
                false
            }
        }
    }

    /// Replace the document with a blank one and reset navigation/selection.
    fn reset_to_new_document(&mut self) {
        while !self.kernel_worker.drain().is_empty() {}

        if let Ok(wb) = self.registry.workbench_mut(&self.active_workbench.0) {
            let cam_pos = self.camera.position();
            let cam_target = self.camera.target();
            let vp = self.camera.viewport_info();
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut self.document,
                cam_pos,
                cam_target,
                (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            );
            wb.on_deactivate(&mut ctx);
            Self::flush_logs(ctx.drain_logs());
        }

        self.document = Document::new("Untitled");
        self.current_file = None;
        self.active_document_object = None;
        self.active_body_id = None;
        self.tree_selection = Some(TreeItemId::DocumentRoot);
        self.selected_body = None;
        self.hovered_body = None;

        let wb_id = self.active_workbench.0.clone();
        self.call_workbench_activate(&wb_id);

        self.camera
            .reset_to_fit(Vec3::ZERO, 50.0, None, &self.user_settings.camera);
        app_log::info("New document");
    }
}

impl ApplicationHandler for PrintCadApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
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

        let window_id = window.id();
        self.ui_layer = Some(UiLayer::new(&window));
        self.gpu_name = renderer.gpu_name().map(|s| s.to_string());
        if let Some(list) = renderer.available_gpus() {
            self.available_gpus = list.to_vec();
        }
        self.renderer = Some(renderer);
        let size = window.inner_size();
        self.camera
            .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
        self.window = Some(window);
        self.window_id = Some(window_id);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if Some(window_id) != self.window_id {
            return;
        }

        // Update picking + viewport-local cursor *before* egui. Cursor events can be marked
        // consumed while dragging UI; we still need consistent coords for 3D hit testing and
        // zoom-to-focal-plane math.
        if let WindowEvent::CursorMoved { position, .. } = &event {
            // `CursorMoved` is already [`PhysicalPosition`]; match renderer + viewport_rect.
            let phys_x = position.x.max(0.0).round() as u32;
            let phys_y = position.y.max(0.0).round() as u32;

            if let Some(renderer) = self.renderer.as_mut() {
                renderer.request_pick(phys_x, phys_y);
            }

            let vp = self.camera.viewport_info();
            let cursor_x = phys_x as f32 - vp.0;
            let cursor_y = phys_y as f32 - vp.1;

            if cursor_x >= 0.0
                && cursor_y >= 0.0
                && cursor_x < vp.2 as f32
                && cursor_y < vp.3 as f32
            {
                self.cursor_in_viewport = Some((cursor_x, cursor_y));
            } else {
                self.cursor_in_viewport = None;
            }
        }

        let vp_cursor = self.cursor_in_viewport.map(|p| Vec2::new(p.0, p.1));
        self.camera.set_cursor_viewport(vp_cursor);

        let zoom_wheel_over_viewport = matches!(event, WindowEvent::MouseWheel { .. })
            && self.cursor_in_viewport.is_some();

        if let (Some(ui_layer), Some(window)) = (self.ui_layer.as_mut(), self.window.as_ref()) {
            let response = ui_layer.on_window_event(window, &event);
            if response.repaint {
                window.request_redraw();
            }
            // egui-winit marks MouseWheel consumed when `wants_pointer_input()` — true over most
            // of the central panel — which prevented the CAD camera from ever seeing scroll.
            if response.consumed && !zoom_wheel_over_viewport {
                return;
            }
        }

        use winit::keyboard::Key;
        if let WindowEvent::KeyboardInput { event: ke, .. } = &event {
            if matches!(ke.state, ElementState::Pressed) {
                if let Key::Character(ch) = &ke.logical_key {
                    let s = ch.as_str();
                    if matches!(s, "h" | "H") && self.cursor_in_viewport.is_some() {
                        if self
                            .camera
                            .pivot_from_key_h(&self.user_settings.camera)
                        {
                            if let Some(w) = self.window.as_ref() {
                                w.request_redraw();
                            }
                        }
                    }
                }
            }
        }

        if let WindowEvent::MouseInput {
            button: MouseButton::Middle,
            state: ElementState::Pressed,
            ..
        } = &event
        {
            if self.cursor_in_viewport.is_some() {
                let hit = self.hovered_world_pos.map(Vec3::from_array);
                self.camera.on_mmb_pivot_pick(hit, &self.user_settings.camera);
                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
        }

        let wb = self.dispatch_workbench_input_without_select(&event);
        let mut redraw = wb.redraw;
        if wb.consumed {
            if redraw {
                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }
            return;
        }

        let orbit_pick = self.hovered_world_pos.map(Vec3::from_array);
        let cam_res = self.camera.on_viewport_pointer(
            &event,
            &self.user_settings.camera,
            orbit_pick,
        );
        redraw |= cam_res.wants_redraw();
        if matches!(cam_res, CameraPointerResult::LmbReleasedMaybeSelect) {
            redraw |= self.toggle_body_under_cursor_selection();
        }

        if redraw {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                if self.confirm_discard_or_save() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    renderer.resize(size);
                }
                self.camera
                    .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                if let Some(window) = self.window.as_ref() {
                    let size = window.inner_size();
                    let _ = inner_size_writer.request_inner_size(size);
                    if let Some(renderer) = self.renderer.as_mut() {
                        renderer.resize(size);
                    }
                    self.camera
                        .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // Optional FPS cap from settings (0 = uncapped).
        // We only advance timing/FPS when we actually render a frame.
        let fps_cap = self.user_settings.fps_cap.max(0.0);
        if fps_cap > 0.0 {
            let target = Duration::from_secs_f32(1.0 / fps_cap);
            if let Some(last) = self.last_frame_time {
                let elapsed = now - last;
                if elapsed < target {
                    let wait_until = last + target;
                    event_loop.set_control_flow(ControlFlow::WaitUntil(wait_until));
                    return;
                }
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(now + target));
        } else {
            // Uncapped: run as fast as possible; vsync/driver may still limit FPS.
            event_loop.set_control_flow(ControlFlow::Poll);
        }

        // Time since last *rendered* frame
        let dt_secs = if let Some(last) = self.last_frame_time {
            let elapsed = now - last;
            let dt = elapsed.as_secs_f32();

            // FPS smoothing: accumulate over ~1s and update display once per second.
            if dt > 0.0 {
                self.fps_accum_time += dt;
                self.fps_frame_count += 1;
                if self.fps_accum_time >= 1.0 {
                    self.current_fps = self.fps_frame_count as f32 / self.fps_accum_time.max(1e-3);
                    self.fps_accum_time = 0.0;
                    self.fps_frame_count = 0;
                }
            }
            dt
        } else {
            0.016 // ~60fps default for first frame
        };

        self.last_frame_time = Some(now);

        let mut workbench_change: Option<(ActiveWorkbench, ActiveWorkbench)> = None;
        let mut new_body_requested_flag = false;
        let mut quit_requested = false;

        // Pull any STEP imports that the kernel worker finished off the
        // queue before we build this frame's submission, so freshly imported
        // bodies show up immediately and the import log lines stay tied to
        // the frame they actually became visible in. Has to happen before
        // we take a mutable borrow on `self.renderer` below.
        self.drain_kernel_responses();

        let (window, renderer) = match (self.window.as_ref(), self.renderer.as_mut()) {
            (Some(window), Some(renderer)) => (window, renderer),
            _ => return,
        };

        // Update camera animation
        self.camera
            .flush_pending_wheel(&self.user_settings.camera);
        self.camera
            .apply_auto_clip_planes(&self.user_settings.camera);
        self.camera
            .update(dt_secs, &self.user_settings.camera);

        // Collect sketch features from document and convert to meshes.
        //
        // Sketch geometry is recomputed every frame (it's cheap), but we
        // bump a per-feature revision based on the underlying JSON so the
        // renderer's cache only re-uploads when the sketch actually changes.
        let sketch_meshes: Vec<BodySubmission> = self
            .document
            .feature_tree()
            .all_nodes()
            .filter_map(|(feature_id, node)| {
                if node.workbench_id.as_str() != "wb.sketch" {
                    return None;
                }

                let sketch_feature = wb_sketch::SketchFeature::from_json(&node.data).ok()?;

                let mesh = wb_sketch::render::sketch_to_mesh(
                    &sketch_feature.sketch,
                    &sketch_feature.plane,
                );

                // Hash the serialized sketch JSON for a stable revision: the
                // renderer skips the upload when the sketch is unchanged.
                let revision = hash_revision(&node.data);

                Some(BodySubmission {
                    id: feature_id.0,
                    revision,
                    mesh: Arc::new(mesh),
                    color: [0.2, 0.8, 0.2],
                    highlight: HighlightState::None,
                    is_wireframe: false,
                })
            })
            .collect();

        // Imported geometry (e.g. STEP files) becomes regular renderable bodies.
        // The body id from the document is reused so picking/selection stays
        // stable, and the document's revision counter is forwarded to the
        // renderer so panning/orbiting never re-uploads the static mesh.
        let imported_meshes: Vec<BodySubmission> = self
            .document
            .imported_geometries()
            .map(|(body_id, geometry)| {
                let is_selected = self.selected_body == Some(body_id.0);
                let is_hovered = self.hovered_body == Some(body_id.0);
                let highlight = match (is_selected, is_hovered) {
                    (true, true) => HighlightState::HoveredAndSelected,
                    (true, false) => HighlightState::Selected,
                    (false, true) => HighlightState::Hovered,
                    (false, false) => HighlightState::None,
                };
                let use_vertex_albedo = geometry.mesh.colors.len() == geometry.mesh.positions.len()
                    && !geometry.mesh.colors.is_empty();
                let base_color = if use_vertex_albedo {
                    [1.0, 1.0, 1.0]
                } else {
                    [0.78, 0.78, 0.82]
                };
                BodySubmission {
                    id: body_id.0,
                    revision: geometry.revision,
                    mesh: Arc::clone(&geometry.mesh),
                    color: base_color,
                    highlight,
                    is_wireframe: false,
                }
            })
            .collect();

        // Get overlay meshes from the active workbench (grid lines, guides, etc.)
        let mut overlay_meshes: Vec<BodySubmission> =
            if let Ok(wb) = self.registry.workbench_mut(&self.active_workbench.0) {
                // Build runtime context for overlay generation
                let cam_pos = self.camera.position();
                let cam_target = self.camera.target();
                let viewport = if let Some(rect) = self.frame_submission.viewport_rect {
                    (rect.x, rect.y, rect.width, rect.height)
                } else {
                    (0, 0, 1920, 1080) // Fallback
                };
                let mut wb_ctx =
                    WorkbenchRuntimeContext::new(&mut self.document, cam_pos, cam_target, viewport);
                wb_ctx.active_document_object = self.active_document_object;
                wb_ctx.selected_body_id = self.active_body_id.map(|id| id.0);

                wb.get_overlay_meshes(&wb_ctx, self.active_document_object)
                    .into_iter()
                    .map(|(mesh, color, is_wireframe)| BodySubmission {
                        // Overlays are regenerated every frame and we don't
                        // have a stable identity for them, so we accept the
                        // upload-and-GC cost: a fresh UUID guarantees a cache
                        // miss this frame, and the GC pass at end-of-frame
                        // reaps the previous frame's entry. Geometry is
                        // typically tiny (grid lines, guides), so this stays
                        // cheap relative to the imported-mesh path.
                        id: Uuid::new_v4(),
                        revision: 0,
                        mesh: Arc::new(mesh),
                        color,
                        highlight: HighlightState::None,
                        is_wireframe,
                    })
                    .collect()
            } else {
                Vec::new()
            };

        // Get screen-space overlays from the active workbench (constant-thickness lines)
        let screen_space_overlays: Vec<core_document::ScreenSpaceOverlay> =
            if let Ok(wb) = self.registry.workbench_mut(&self.active_workbench.0) {
                // Build runtime context for overlay generation
                let cam_pos = self.camera.position();
                let cam_target = self.camera.target();
                let viewport = if let Some(rect) = self.frame_submission.viewport_rect {
                    (rect.x, rect.y, rect.width, rect.height)
                } else {
                    (0, 0, 1920, 1080) // Fallback
                };
                let mut wb_ctx =
                    WorkbenchRuntimeContext::new(&mut self.document, cam_pos, cam_target, viewport);
                wb_ctx.active_document_object = self.active_document_object;
                wb_ctx.selected_body_id = self.active_body_id.map(|id| id.0);
                wb_ctx.view_proj = Some(self.camera.view_projection());

                wb.get_screen_space_overlays(&wb_ctx, self.active_document_object)
            } else {
                Vec::new()
            };

        // Combine sketch meshes, imported geometry, and overlay meshes.
        let mut all_meshes = sketch_meshes;
        all_meshes.extend(imported_meshes);
        all_meshes.append(&mut overlay_meshes);

        self.frame_submission.bodies = all_meshes;
        self.frame_submission.view_proj = self.camera.view_projection();
        self.frame_submission.camera_pos = self.camera.position();
        self.frame_submission.lighting = lighting_data_from_settings(&self.user_settings.lighting);
        self.frame_submission.screen_space_overlays = screen_space_overlays;

        let mut ui_result_open = false;
        let mut ui_result_new = false;
        let mut ui_result_save = false;
        let mut ui_result_save_as = false;
        let mut ui_result_import_step = false;

        if let Some(ui_layer) = self.ui_layer.as_mut() {
            let orientation_input = OrientationCubeInput {
                camera_orientation: self.camera.orientation(),
                axis_system: self.camera.axis_system(),
            };

            let pivot_screen_pos = self.camera.rotation_pivot_indicator_screen_px(
                self.user_settings.camera.orbit_pivot_pick,
            );

            let ui_result = ui_layer.run(
                window,
                &mut self.user_settings,
                Some(&orientation_input),
                self.current_fps,
                self.gpu_name.as_deref(),
                &self.available_gpus,
                self.hovered_world_pos,
                pivot_screen_pos,
                self.camera.axis_system(),
                &mut self.document,
                &mut self.registry,
                self.tree_selection,
                self.active_document_object,
                self.active_body_id,
                &self.frame_submission.screen_space_overlays,
                self.kernel_worker.in_flight(),
            );
            self.frame_submission.egui = Some(ui_result.submission);
            self.active_tool = ui_result.active_tool;

            // Track workbench change
            if ui_result.workbench_changed {
                workbench_change = Some((
                    self.active_workbench.clone(),
                    ui_result.active_workbench.clone(),
                ));
            }
            self.active_workbench = ui_result.active_workbench;

            self.frame_submission.viewport_rect = Some(RenderViewportRect {
                x: ui_result.viewport.x,
                y: ui_result.viewport.y,
                width: ui_result.viewport.width,
                height: ui_result.viewport.height,
            });
            self.camera.update_viewport(
                (ui_result.viewport.x, ui_result.viewport.y),
                (
                    ui_result.viewport.width.max(1),
                    ui_result.viewport.height.max(1),
                ),
            );

            // Handle orientation cube interactions
            if let Some(snap_view) = ui_result.snap_to_view {
                self.camera
                    .snap_to_view(snap_view, &self.user_settings.camera);
            }
            if let Some(ref rotate_delta) = ui_result.rotate_delta {
                self.camera
                    .apply_rotate_delta(rotate_delta, &self.user_settings.camera);
            }

            if ui_result.settings_changed {
                self.camera.sync_with_settings(&self.user_settings.camera);
                if let Err(err) = self.settings_store.save(&self.user_settings) {
                    app_log::warn(format!("Failed to save settings: {err}"));
                }
            }

            // The Part Design workbench exposes "New Body" as an Action tool.
            // Action tools live in `active_ids` for exactly one frame; we
            // detect a fresh click here and consume it so the body-creation
            // call (deferred until after the renderer borrow ends) only
            // fires once.
            if self.active_tool.active_ids.remove("part.new_body") {
                new_body_requested_flag = true;
            }

            ui_result_open = ui_result.open_requested;
            ui_result_new = ui_result.new_requested;
            ui_result_save = ui_result.save_requested;
            ui_result_save_as = ui_result.save_as_requested;
            ui_result_import_step = ui_result.import_step_requested;
            quit_requested = ui_result.quit_requested;

            if ui_result.reset_view_requested {
                app_log::info("Fit View requested");
                if let Some(aabb) = document_imported_aabb(&self.document) {
                    let (center, radius) = aabb_fit_center_radius(aabb.0, aabb.1);
                    self.camera.reset_to_fit(
                        center,
                        radius,
                        Some(aabb),
                        &self.user_settings.camera,
                    );
                } else {
                    self.camera.reset_to_fit(
                        Vec3::ZERO,
                        50.0,
                        None,
                        &self.user_settings.camera,
                    );
                }
            }

            if ui_result.finish_sketch_requested {
                // Defer handling until after rendering to avoid borrow conflicts.
                // We'll process this flag once we exit the UI closure.
            }

            if let Some(selection) = ui_result.tree_selection {
                self.tree_selection = Some(selection);
                match selection {
                    TreeItemId::DocumentRoot => {
                        self.active_document_object = None;
                        self.active_body_id = None;
                        self.selected_body = None;
                    }
                    TreeItemId::Body(id) => {
                        self.active_body_id = Some(id);
                        self.active_document_object = None;
                        self.selected_body = Some(id.0);
                    }
                    TreeItemId::Feature(id) => {
                        if self.active_document_object != Some(id) {
                            app_log::info(format!("Selected feature {:?}", id));
                        }
                        self.active_document_object = Some(id);
                    }
                }
            }

            if let Some(item) = ui_result.tree_activation {
                match item {
                    TreeItemId::Feature(id) => {
                        app_log::info(format!("Activated feature {:?} (double-click in tree)", id));
                    }
                    TreeItemId::Body(id) => {
                        app_log::info(format!("Activated body {:?} (double-click in tree)", id));
                    }
                    TreeItemId::DocumentRoot => {}
                }
            }
        } else {
            self.frame_submission.egui = None;
            self.frame_submission.viewport_rect = None;
        }

        window.request_redraw();

        if let Err(err) = renderer.render(&self.frame_submission) {
            app_log::error(format!("Render failure: {err}"));
            event_loop.exit();
            return;
        }

        // Retrieve pick result from GPU picking (processed during render)
        let pick_result = renderer.pick_at(0, 0); // Coordinates don't matter, we use cached result
        self.hovered_body = pick_result.body_id;
        self.hovered_world_pos = pick_result.world_position;

        if ui_result_new && self.confirm_discard_or_save() {
            self.reset_to_new_document();
        }

        let open_after_confirm = ui_result_open && self.confirm_discard_or_save();

        if open_after_confirm || ui_result_save || ui_result_save_as || ui_result_import_step {
            self.start_file_dialog(
                open_after_confirm,
                ui_result_save,
                ui_result_save_as,
                ui_result_import_step,
            );
        }

        if let Some(rx) = &self.file_dialog_rx {
            if let Ok(result) = rx.try_recv() {
                match result.kind {
                    FileDialogKind::Open => {
                        if let Some(path) = result.path {
                            if let Err(err) = self.open_document_at(&path) {
                                app_log::error(format!("Failed to open document: {err}"));
                            }
                        }
                    }
                    FileDialogKind::Save => {
                        if let Some(path) = result.path {
                            if let Err(err) = self.save_document_at(&path) {
                                app_log::error(format!("Failed to save document: {err}"));
                            }
                        }
                    }
                    FileDialogKind::SaveAs => {
                        if let Some(path) = result.path {
                            if let Err(err) = self.save_document_at(&path) {
                                app_log::error(format!("Failed to save document: {err}"));
                            }
                        }
                    }
                    FileDialogKind::ImportStep => {
                        if let Some(path) = result.path {
                            self.import_step_at(&path);
                        }
                    }
                }
                self.file_dialog_rx = None;
            }
        }

        if new_body_requested_flag {
            self.create_new_body();
        }

        // Now handle workbench change (after renderer borrow ends)
        if let Some((old_wb, new_wb)) = workbench_change {
            self.call_workbench_deactivate(&old_wb.0);

            self.call_workbench_activate(&new_wb.0);
        }

        // File > Quit / Ctrl+Q. Deferred to here so the rest of the frame
        // (rendering, picks, dialogs) finishes cleanly before the loop ends.
        if quit_requested && self.confirm_discard_or_save() {
            app_log::info("Quit requested via menu / shortcut");
            event_loop.exit();
        }
    }
}

impl PrintCadApp {
    fn create_new_body(&mut self) {
        let body_id = self.document.create_body(None);
        if let Some(body) = self.document.bodies().iter().find(|b| b.id == body_id) {
            app_log::info(format!("Created {}", body.name));
        } else {
            app_log::info(format!("Created body {:?}", body_id));
        }
        self.active_body_id = Some(body_id);
        self.active_document_object = None;
        self.tree_selection = Some(TreeItemId::Body(body_id));
        self.selected_body = Some(body_id.0);
    }

    fn open_document_at(&mut self, path: &PathBuf) -> Result<()> {
        // Support legacy .json files directly, otherwise use the .prtcad tar-based format.
        let document = match path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ext) if ext == "json" => {
                let file = std::fs::File::open(path)
                    .with_context(|| format!("Failed to open document file {}", path.display()))?;
                serde_json::from_reader(file).with_context(|| "Failed to parse document JSON")?
            }
            _ => Document::load_from_file(path)
                .with_context(|| format!("Failed to open .prtcad document {}", path.display()))?,
        };

        self.document = document;
        self.current_file = Some(path.clone());
        // Derive a user-facing document name from the file name (strip known extensions).
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let lowered = file_name.to_ascii_lowercase();
        let name = if let Some(stripped) = lowered.strip_suffix(".prtcad.zst") {
            &file_name[..stripped.len()]
        } else if let Some(stripped) = lowered.strip_suffix(".prtcad.gz") {
            &file_name[..stripped.len()]
        } else if let Some(stripped) = lowered.strip_suffix(".prtcad") {
            &file_name[..stripped.len()]
        } else if let Some(stripped) = lowered.strip_suffix(".json") {
            &file_name[..stripped.len()]
        } else {
            file_name
        };
        self.document.set_name(name);
        self.active_document_object = None;
        self.active_body_id = None;
        self.tree_selection = Some(TreeItemId::DocumentRoot);
        self.selected_body = None;

        self.document.mark_clean();
        Self::write_recent_dir(path);
        // Match STEP import: reframe imported mesh bounds so scene AABB and auto
        // near/far use the same view as STEP apply (opening only updated zoom
        // limits before, which left stale eye/target → marginal clipping until the
        // user toggled projection or hit Fit View).
        if let Some((mn, mx)) = document_imported_aabb(&self.document) {
            let (center, radius) = aabb_fit_center_radius(mn, mx);
            self.camera.reset_to_fit(
                center,
                radius,
                Some((mn, mx)),
                &self.user_settings.camera,
            );
        } else {
            self.camera.clear_scene_zoom_constraint();
            self.camera
                .clamp_focal_to_settings(&self.user_settings.camera);
        }
        app_log::info(format!("Opened document from {}", path.display()));
        Ok(())
    }

    fn save_document_at(&mut self, path: &PathBuf) -> Result<()> {
        // Derive a user-facing document name from the file name (strip known extensions).
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("Untitled");
        let lowered = file_name.to_ascii_lowercase();
        let name = if let Some(stripped) = lowered.strip_suffix(".prtcad.zst") {
            &file_name[..stripped.len()]
        } else if let Some(stripped) = lowered.strip_suffix(".prtcad.gz") {
            &file_name[..stripped.len()]
        } else if let Some(stripped) = lowered.strip_suffix(".prtcad") {
            &file_name[..stripped.len()]
        } else if let Some(stripped) = lowered.strip_suffix(".json") {
            &file_name[..stripped.len()]
        } else {
            file_name
        };
        self.document.set_name(name);

        let ext_lower = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase());
        if matches!(ext_lower.as_deref(), Some("json")) && self.document_has_asset_files() {
            let _ = MessageDialog::new()
                .set_title("Cannot save as JSON")
                .set_description(
                    "This document has embedded assets (e.g. imported STEP). JSON export does not include those bytes. Save as .prtcad instead.",
                )
                .set_level(MessageLevel::Warning)
                .set_buttons(MessageButtons::Ok)
                .show();
            return Err(anyhow::anyhow!(
                "JSON format cannot store embedded assets; save as .prtcad"
            ));
        }

        // For legacy .json files, keep writing plain JSON.
        // For everything else, use the .prtcad tar-based container with optional compression.
        match path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
        {
            Some(ext) if ext == "json" => {
                let file = std::fs::File::create(path).with_context(|| {
                    format!("Failed to create document file {}", path.display())
                })?;
                serde_json::to_writer_pretty(file, &self.document)
                    .with_context(|| "Failed to serialize document")?;
            }
            _ => {
                // Choose compression based on the full file name suffix.
                let compression = if lowered.ends_with(".prtcad.gz") || lowered.ends_with(".gz") {
                    core_document::Compression::Gzip
                } else if lowered.ends_with(".prtcad.zst") || lowered.ends_with(".zst") {
                    core_document::Compression::Zstd
                } else {
                    core_document::Compression::None
                };

                self.document
                    .save_to_file(path, compression)
                    .with_context(|| {
                        format!("Failed to save .prtcad document {}", path.display())
                    })?;
            }
        }

        self.current_file = Some(path.clone());
        Self::write_recent_dir(path);
        self.document.mark_clean();
        app_log::info(format!("Saved document to {}", path.display()));
        Ok(())
    }

    fn start_file_dialog(
        &mut self,
        open: bool,
        _save: bool,
        save_as: bool,
        import_step: bool,
    ) {
        use std::sync::mpsc;
        if self.file_dialog_rx.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel::<FileDialogResult>();
        self.file_dialog_rx = Some(rx);

        let kind = if import_step {
            FileDialogKind::ImportStep
        } else if open {
            FileDialogKind::Open
        } else if save_as {
            FileDialogKind::SaveAs
        } else {
            FileDialogKind::Save
        };

        let current_path = self.current_file.clone();

        std::thread::spawn(move || {
            let mut dialog = match kind {
                FileDialogKind::ImportStep => {
                    rfd::FileDialog::new().add_filter("STEP file", &["step", "stp"])
                }
                _ => rfd::FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]),
            };

            if let Ok(recent_path) = settings::SettingsStore::recent_file_path() {
                if let Ok(file) = std::fs::File::open(&recent_path) {
                    if let Ok(saved_dir_str) = serde_json::from_reader::<_, String>(file) {
                        let saved_dir = std::path::PathBuf::from(saved_dir_str);
                        dialog = dialog.set_directory(saved_dir);
                    }
                }
            }

            let path = match kind {
                FileDialogKind::Open => dialog.pick_file(),
                FileDialogKind::ImportStep => dialog.pick_file(),
                FileDialogKind::Save => {
                    if let Some(existing) = current_path {
                        Some(existing)
                    } else {
                        dialog.set_file_name("untitled.prtcad").save_file()
                    }
                }
                FileDialogKind::SaveAs => dialog.set_file_name("untitled.prtcad").save_file(),
            };

            let _ = tx.send(FileDialogResult { kind, path });
        });
    }

    /// Submit a STEP/STP import to the kernel worker. Returns immediately;
    /// the response is delivered later via `drain_kernel_responses` and the
    /// document mutation happens in `apply_step_import` once the worker is
    /// done. Logging the start/finish here keeps the user oriented while the
    /// import is in flight.
    fn import_step_at(&mut self, path: &PathBuf) {
        let detail = TessellationSettings::default();
        app_log::info(format!("Importing STEP `{}`...", path.display()));
        self.kernel_worker
            .request_step_import(path.clone(), detail);
    }

    /// Drain any STEP responses that have arrived from the kernel worker and
    /// fold them into the document. Called once per frame in `about_to_wait`.
    fn drain_kernel_responses(&mut self) {
        for response in self.kernel_worker.drain() {
            match response {
                KernelResponse::StepImported {
                    path,
                    model,
                    raw_bytes,
                    elapsed,
                } => {
                    if let Err(err) = self.apply_step_import(&path, model, raw_bytes, elapsed) {
                        app_log::error(format!(
                            "Failed to apply STEP import {}: {err}",
                            path.display()
                        ));
                    }
                }
                KernelResponse::StepFailed { path, error } => {
                    app_log::error(format!("STEP import failed `{}`: {}", path.display(), error));
                }
            }
        }
    }

    /// Register the imported bodies + raw asset bytes on the document and
    /// frame the camera around the new geometry. Mirrors the behaviour of
    /// the previous synchronous `import_step_at` but runs entirely on the UI
    /// thread after the heavy CPU work has completed in the worker.
    fn apply_step_import(
        &mut self,
        path: &PathBuf,
        imported: ImportedModel,
        raw_bytes: Vec<u8>,
        elapsed: Duration,
    ) -> Result<()> {
        let apply_start = Instant::now();
        // Capture "fresh document" *before* we start mutating it so the
        // auto-unit pick below isn't confused by bodies we're about to add.
        let was_fresh_document = self.document.bodies().is_empty()
            && !self.document.assets().any(|_| true)
            && self.document.imported_geometries().next().is_none();

        if imported.bodies.is_empty() {
            app_log::warn(format!(
                "STEP import produced no geometry: {}",
                path.display()
            ));
            return Ok(());
        }

        let detected_unit = imported.source_unit.map(length_unit_to_document_unit);

        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_else(|| "step".to_string());
        let asset = core_document::AssetReference::new(
            format!("assets/{}.{}", uuid::Uuid::new_v4(), extension),
            core_document::AssetType::Step,
            serde_json::json!({
                "source_path": path.display().to_string(),
                "body_count": imported.bodies.len(),
            }),
        );
        let asset_id = self.document.add_asset_with_data(asset, raw_bytes);

        let mut total_triangles: usize = 0;
        let mut combined_min = [f32::INFINITY; 3];
        let mut combined_max = [f32::NEG_INFINITY; 3];
        let mut first_body: Option<BodyId> = None;

        for body in imported.bodies {
            let body_id = self.document.create_body(body.name.clone());
            if first_body.is_none() {
                first_body = Some(body_id);
            }
            total_triangles += body.mesh.indices.len() / 3;
            if let Some((min, max)) = body.mesh.bounds() {
                for axis in 0..3 {
                    combined_min[axis] = combined_min[axis].min(min[axis]);
                    combined_max[axis] = combined_max[axis].max(max[axis]);
                }
            }
            self.document.set_imported_geometry(
                body_id,
                ImportedGeometry {
                    mesh: Arc::new(body.mesh),
                    source_asset: Some(asset_id),
                    // `set_imported_geometry` overwrites this; any value works.
                    revision: 0,
                },
            );
        }

        if combined_min[0] <= combined_max[0] {
            let aabb_min = Vec3::new(combined_min[0], combined_min[1], combined_min[2]);
            let aabb_max = Vec3::new(combined_max[0], combined_max[1], combined_max[2]);
            let (center, radius) = aabb_fit_center_radius(aabb_min, aabb_max);
            self.camera.reset_to_fit(
                center,
                radius,
                Some((aabb_min, aabb_max)),
                &self.user_settings.camera,
            );
        }

        if let Some(body_id) = first_body {
            self.active_body_id = Some(body_id);
            self.tree_selection = Some(TreeItemId::Body(body_id));
            self.selected_body = Some(body_id.0);
        }

        // On a fresh document, adopt the STEP file's declared unit as the
        // document's display unit. Otherwise leave the user's choice intact —
        // mixing two STEP files with different units shouldn't silently flip
        // the active document's display.
        if was_fresh_document {
            if let Some(unit) = detected_unit {
                self.document.set_display_unit(unit);
                app_log::info(format!(
                    "Display unit set to {} from imported STEP `{}`",
                    unit.short_label(),
                    path.display()
                ));
            }
        }

        Self::write_recent_dir(path);
        let apply_ms = apply_start.elapsed().as_secs_f64() * 1000.0;
        info!(
            path = %path.display(),
            apply_to_document_ms = format!("{apply_ms:.2}"),
            worker_elapsed_ms = format!("{:.2}", elapsed.as_secs_f64() * 1000.0),
            "STEP import timing (UI thread apply)"
        );
        app_log::info(format!(
            "Imported STEP `{}` in {:.0}ms worker + {:.1}ms apply: {} bodies, {} triangles",
            path.display(),
            elapsed.as_secs_f64() * 1000.0,
            apply_ms,
            self.document
                .imported_geometries()
                .filter(|(_, g)| g.source_asset == Some(asset_id))
                .count(),
            total_triangles
        ));

        Ok(())
    }

    fn write_recent_dir(path: &PathBuf) {
        if let Ok(recent_path) = settings::SettingsStore::recent_file_path() {
            if let Some(dir) = path.parent() {
                if let Ok(file) = std::fs::File::create(&recent_path) {
                    let mut s = dir.to_string_lossy().to_string();
                    if !s.ends_with(std::path::MAIN_SEPARATOR) {
                        s.push(std::path::MAIN_SEPARATOR);
                    }
                    let _ = serde_json::to_writer(file, &s);
                }
            }
        }
    }

    fn dispatch_workbench_input_without_select(
        &mut self,
        event: &WindowEvent,
    ) -> core_document::InputResult {
        let wb_event = match self.convert_to_wb_event(event) {
            Some(e) => e,
            None => return core_document::InputResult::ignored(),
        };

        let wb_id = self.active_workbench_id();
        let active_tool_id = self.active_tool.active_ids.iter().next().cloned();
        let active_tool_str = active_tool_id.as_deref();
        let result = self.call_workbench_input(&wb_id, &wb_event, active_tool_str);

        if let Some(tool_id) = active_tool_id {
            if tool_id == "sketch.create" && result.consumed {
                self.active_tool.active_ids.remove(&tool_id);
            }
        }

        result
    }



    /// Call on_input on a workbench.
    fn call_workbench_input(
        &mut self,
        wb_id: &WorkbenchId,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
    ) -> core_document::InputResult {
        // Collect camera/viewport info first
        let cam_pos = self.camera.position();
        let cam_target = self.camera.target();
        let vp = self.camera.viewport_info();
        let mut hovered_world_pos = self.hovered_world_pos;
        let hovered_body_id = self.hovered_body;
        let selected_body_id = self.selected_body;
        let cursor_viewport_pos = self.cursor_in_viewport;

        // For sketch workbench, if we have a mouse event with viewport coordinates
        // and no hovered world position, try to project onto the active sketch plane
        if wb_id.as_str() == "wb.sketch" {
            if let WorkbenchInputEvent::MousePress { viewport_pos, .. } = event {
                if hovered_world_pos.is_none() {
                    // Try to get active sketch plane from document
                    if let Some((_, node)) = self
                        .document
                        .feature_tree()
                        .all_nodes()
                        .find(|(_, n)| n.workbench_id.as_str() == "wb.sketch")
                    {
                        if let Ok(sketch_feature) = wb_sketch::SketchFeature::from_json(&node.data)
                        {
                            let plane_origin = glam::Vec3::from_array(sketch_feature.plane.origin);
                            let plane_normal = glam::Vec3::from_array(sketch_feature.plane.normal);

                            // Use viewport-local coordinates directly to project onto the sketch plane.
                            if let Some(world_pos) = self.camera.viewport_to_plane(
                                viewport_pos.0,
                                viewport_pos.1,
                                plane_origin,
                                plane_normal,
                            ) {
                                app_log::info(format!(
                                    "Sketch raycast: viewport=({:.1}, {:.1}) -> world=({:.3}, {:.3}, {:.3})",
                                    viewport_pos.0,
                                    viewport_pos.1,
                                    world_pos.x,
                                    world_pos.y,
                                    world_pos.z
                                ));
                                hovered_world_pos = Some(world_pos.to_array());
                            }
                        }
                    }
                }
            }
        }

        // Get workbench and call hook
        if let Ok(wb) = self.registry.workbench_mut(wb_id) {
            let mut ctx = WorkbenchRuntimeContext::new(
                &mut self.document,
                cam_pos,
                cam_target,
                (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            );
            ctx.hovered_world_pos = hovered_world_pos;
            ctx.hovered_body_id = hovered_body_id;
            ctx.selected_body_id = selected_body_id;
            ctx.cursor_viewport_pos = cursor_viewport_pos;
            ctx.active_document_object = self.active_document_object;

            let result = wb.on_input(event, active_tool, &mut ctx);

            // Sync active_document_object from context (workbench may have set it)
            if ctx.active_document_object != self.active_document_object {
                self.active_document_object = ctx.active_document_object;
            }

            // Handle camera orientation request
            if let Some(orient_req) = ctx.camera_orient_request.take() {
                self.camera.orient_to_plane(
                    glam::Vec3::from_array(orient_req.plane_origin),
                    glam::Vec3::from_array(orient_req.plane_normal),
                    glam::Vec3::from_array(orient_req.plane_up),
                    &self.user_settings.camera,
                );
            }

            Self::flush_logs(ctx.drain_logs());
            result
        } else {
            core_document::InputResult::ignored()
        }
    }

    /// Convert a winit WindowEvent to a WorkbenchInputEvent.
    fn convert_to_wb_event(&self, event: &WindowEvent) -> Option<WorkbenchInputEvent> {
        match event {
            WindowEvent::MouseInput { state, button, .. } => {
                let wb_button = match button {
                    MouseButton::Left => WbMouseButton::Left,
                    MouseButton::Middle => WbMouseButton::Middle,
                    MouseButton::Right => WbMouseButton::Right,
                    MouseButton::Other(n) => WbMouseButton::Other(*n),
                    _ => return None,
                };
                let viewport_pos = self.cursor_in_viewport.unwrap_or((0.0, 0.0));
                match state {
                    ElementState::Pressed => Some(WorkbenchInputEvent::MousePress {
                        button: wb_button,
                        viewport_pos,
                    }),
                    ElementState::Released => Some(WorkbenchInputEvent::MouseRelease {
                        button: wb_button,
                        viewport_pos,
                    }),
                }
            }
            WindowEvent::CursorMoved { .. } => {
                let viewport_pos = self.cursor_in_viewport?;
                Some(WorkbenchInputEvent::MouseMove { viewport_pos })
            }
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{Key, NamedKey};
                let key = match &event.logical_key {
                    Key::Named(NamedKey::Escape) => core_document::KeyCode::Escape,
                    Key::Named(NamedKey::Enter) => core_document::KeyCode::Enter,
                    Key::Named(NamedKey::Space) => core_document::KeyCode::Space,
                    Key::Named(NamedKey::Delete) => core_document::KeyCode::Delete,
                    Key::Named(NamedKey::Backspace) => core_document::KeyCode::Backspace,
                    Key::Named(NamedKey::Tab) => core_document::KeyCode::Tab,
                    Key::Character(c) => match c.as_str() {
                        "a" | "A" => core_document::KeyCode::A,
                        "b" | "B" => core_document::KeyCode::B,
                        "c" | "C" => core_document::KeyCode::C,
                        "d" | "D" => core_document::KeyCode::D,
                        "e" | "E" => core_document::KeyCode::E,
                        "f" | "F" => core_document::KeyCode::F,
                        "g" | "G" => core_document::KeyCode::G,
                        "h" | "H" => core_document::KeyCode::H,
                        "i" | "I" => core_document::KeyCode::I,
                        "j" | "J" => core_document::KeyCode::J,
                        "k" | "K" => core_document::KeyCode::K,
                        "l" | "L" => core_document::KeyCode::L,
                        "m" | "M" => core_document::KeyCode::M,
                        "n" | "N" => core_document::KeyCode::N,
                        "o" | "O" => core_document::KeyCode::O,
                        "p" | "P" => core_document::KeyCode::P,
                        "q" | "Q" => core_document::KeyCode::Q,
                        "r" | "R" => core_document::KeyCode::R,
                        "s" | "S" => core_document::KeyCode::S,
                        "t" | "T" => core_document::KeyCode::T,
                        "u" | "U" => core_document::KeyCode::U,
                        "v" | "V" => core_document::KeyCode::V,
                        "w" | "W" => core_document::KeyCode::W,
                        "x" | "X" => core_document::KeyCode::X,
                        "y" | "Y" => core_document::KeyCode::Y,
                        "z" | "Z" => core_document::KeyCode::Z,
                        "0" => core_document::KeyCode::Key0,
                        "1" => core_document::KeyCode::Key1,
                        "2" => core_document::KeyCode::Key2,
                        "3" => core_document::KeyCode::Key3,
                        "4" => core_document::KeyCode::Key4,
                        "5" => core_document::KeyCode::Key5,
                        "6" => core_document::KeyCode::Key6,
                        "7" => core_document::KeyCode::Key7,
                        "8" => core_document::KeyCode::Key8,
                        "9" => core_document::KeyCode::Key9,
                        _ => core_document::KeyCode::Unknown,
                    },
                    _ => core_document::KeyCode::Unknown,
                };
                match event.state {
                    ElementState::Pressed => Some(WorkbenchInputEvent::KeyPress { key }),
                    ElementState::Released => Some(WorkbenchInputEvent::KeyRelease { key }),
                }
            }
            _ => None,
        }
    }

    fn toggle_body_under_cursor_selection(&mut self) -> bool {
        if let Some(hovered) = self.hovered_body {
            if self.selected_body == Some(hovered) {
                self.selected_body = None;
                app_log::info("Deselected body");
            } else {
                self.selected_body = Some(hovered);
                app_log::info(format!("Selected body: {hovered:?}"));
            }
        } else if self.selected_body.is_some() {
            self.selected_body = None;
            app_log::info("Deselected (clicked empty space)");
        }
        true
    }

}

fn lighting_data_from_settings(settings: &LightingSettings) -> LightingData {
    LightingData {
        main_light: GpuLight::new(
            settings.main_light.direction(),
            settings.main_light.color,
            settings.main_light.intensity,
            settings.main_light.enabled,
        ),
        backlight: GpuLight::new(
            settings.backlight.direction(),
            settings.backlight.color,
            settings.backlight.intensity,
            settings.backlight.enabled,
        ),
        fill_light: GpuLight::new(
            settings.fill_light.direction(),
            settings.fill_light.color,
            settings.fill_light.intensity,
            settings.fill_light.enabled,
        ),
        ambient_color: settings.ambient_color,
        ambient_intensity: settings.ambient_intensity,
        edge_line_color: settings.edge_line_color,
        edge_line_width: settings.edge_line_width,
    }
}
