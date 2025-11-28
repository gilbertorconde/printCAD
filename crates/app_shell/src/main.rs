mod camera;
mod log_panel;
mod orientation_cube;
mod ui;

use anyhow::{Context, Result};
use camera::CameraController;
use core_document::{
    BodyId, Document, DocumentService, LogLevel, MouseButton as WbMouseButton, WorkbenchFeature,
    WorkbenchId, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use glam::Vec3;
use log_panel as app_log;
use orientation_cube::OrientationCubeInput;
use render_vk::{
    BodySubmission, FrameSubmission, GpuLight, HighlightState, LightingData, RenderBackend,
    RenderSettings, ViewportRect as RenderViewportRect, VulkanRenderer,
};
use settings::{LightingSettings, SettingsStore, UserSettings};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::error;
use ui::{ActiveTool, ActiveWorkbench, TreeItemId, UiLayer};
use uuid::Uuid;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};
use workbenches::register_all_workbenches;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

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

struct PrintCadApp {
    settings: RenderSettings,
    renderer: Option<VulkanRenderer>,
    frame_submission: FrameSubmission,
    window: Option<Window>,
    window_id: Option<WindowId>,
    ui_layer: Option<UiLayer>,
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
}

enum FileDialogKind {
    Open,
    Save,
    SaveAs,
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
            renderer: None,
            frame_submission: FrameSubmission::default(),
            window: None,
            window_id: None,
            ui_layer: None,
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

        if let (Some(ui_layer), Some(window)) = (self.ui_layer.as_mut(), self.window.as_ref()) {
            let response = ui_layer.on_window_event(window, &event);
            if response.repaint {
                window.request_redraw();
            }
            if response.consumed {
                return;
            }
        }

        // Track cursor position for picking (in physical/surface coordinates)
        if let WindowEvent::CursorMoved { position, .. } = &event {
            // Convert logical window coordinates to physical pixels to match the
            // viewport and renderer coordinate spaces.
            let scale = self
                .window
                .as_ref()
                .map(|w| w.scale_factor() as f32)
                .unwrap_or(1.0);
            let phys_x = (position.x as f32 * scale).round() as u32;
            let phys_y = (position.y as f32 * scale).round() as u32;

            // Request GPU picking at cursor position
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.request_pick(phys_x, phys_y);
            }

            // Store cursor position relative to viewport for other uses
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

        if self.handle_tool_input(&event) {
            if let Some(window) = self.window.as_ref() {
                window.request_redraw();
            }
            return;
        }

        if let Some(window) = self.window.as_ref() {
            if self.camera.handle_event(&event, &self.user_settings.camera) {
                window.request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
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

        let mut new_body_requested_flag = false;
        let mut workbench_change: Option<(ActiveWorkbench, ActiveWorkbench)> = None;

        let (window, renderer) = match (self.window.as_ref(), self.renderer.as_mut()) {
            (Some(window), Some(renderer)) => (window, renderer),
            _ => return,
        };

        // Update camera animation
        self.camera.update(dt_secs);

        // Collect sketch features from document and convert to meshes
        let sketch_meshes: Vec<BodySubmission> = self
            .document
            .feature_tree()
            .all_nodes()
            .filter_map(|(feature_id, node)| {
                // Only process sketch features
                if node.workbench_id.as_str() != "wb.sketch" {
                    return None;
                }

                // Deserialize sketch feature
                let sketch_feature = wb_sketch::SketchFeature::from_json(&node.data).ok()?;

                // Convert to mesh
                let mesh = wb_sketch::render::sketch_to_mesh(
                    &sketch_feature.sketch,
                    &sketch_feature.plane,
                );

                // Create body submission for sketch (use feature ID UUID as body ID)
                Some(BodySubmission {
                    id: feature_id.0,
                    mesh,
                    color: [0.2, 0.8, 0.2], // Green color for sketches
                    highlight: HighlightState::None,
                })
            })
            .collect();

        // For now, only render sketch meshes (no demo bodies).
        self.frame_submission.bodies = sketch_meshes;
        self.frame_submission.view_proj = self.camera.view_projection();
        self.frame_submission.camera_pos = self.camera.position();
        self.frame_submission.lighting = lighting_data_from_settings(&self.user_settings.lighting);

        let mut ui_result_open = false;
        let mut ui_result_save = false;
        let mut ui_result_save_as = false;

        if let Some(ui_layer) = self.ui_layer.as_mut() {
            let orientation_input = OrientationCubeInput {
                camera_orientation: self.camera.orientation(),
                axis_system: self.camera.axis_system(),
            };

            // Get pivot screen position for visual indicator
            let pivot_screen_pos = self
                .camera
                .active_pivot()
                .and_then(|pivot| self.camera.world_to_screen(pivot));

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
                self.camera.snap_to_view(snap_view);
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

            if ui_result.new_body_requested {
                new_body_requested_flag = true;
            }
            ui_result_open = ui_result.open_requested;
            ui_result_save = ui_result.save_requested;
            ui_result_save_as = ui_result.save_as_requested;

            if ui_result.reset_view_requested {
                app_log::info("Fit View requested");
                // TODO: compute bounds from real document bodies once available.
                // For now, reset to a reasonable default around the origin.
                use glam::Vec3;
                self.camera.reset_to_fit(Vec3::ZERO, 1.0);
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

        // Set orbit pivot based on what's under the cursor
        // If hovering over geometry, orbit around that point; otherwise use default target
        if let Some(world_pos) = pick_result.world_position {
            self.camera
                .set_orbit_pivot(Some(Vec3::from_array(world_pos)));
        } else {
            self.camera.set_orbit_pivot(None);
        }

        if ui_result_open || ui_result_save || ui_result_save_as {
            self.start_file_dialog(ui_result_open, ui_result_save, ui_result_save_as);
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

        Self::write_recent_dir(path);
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
        app_log::info(format!("Saved document to {}", path.display()));
        Ok(())
    }

    fn start_file_dialog(&mut self, open: bool, _save: bool, save_as: bool) {
        use std::sync::mpsc;
        if self.file_dialog_rx.is_some() {
            return;
        }

        let (tx, rx) = mpsc::channel::<FileDialogResult>();
        self.file_dialog_rx = Some(rx);

        let kind = if open {
            FileDialogKind::Open
        } else if save_as {
            FileDialogKind::SaveAs
        } else {
            FileDialogKind::Save
        };

        let current_path = self.current_file.clone();

        std::thread::spawn(move || {
            let mut dialog =
                rfd::FileDialog::new().add_filter("printCAD Document", &["prtcad", "json"]);

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

    fn handle_tool_input(&mut self, event: &WindowEvent) -> bool {
        // Convert winit event to workbench input event
        let wb_event = match self.convert_to_wb_event(event) {
            Some(e) => e,
            None => return false,
        };

        // First, let the active workbench handle the event
        let wb_id = self.active_workbench_id();
        let active_tool = self.active_tool.id.clone();
        let result = self.call_workbench_input(&wb_id, &wb_event, active_tool.as_deref());

        // Treat "sketch.create" like a momentary action button:
        // once the workbench has consumed the event, clear the active tool so
        // subsequent input events don't keep re-triggering the action.
        if matches!(active_tool.as_deref(), Some("sketch.create")) && result.consumed {
            self.active_tool.id = None;
        }

        if result.consumed {
            return result.redraw;
        }

        // If workbench didn't consume, handle with default behavior (select tool)
        self.handle_select_tool(event)
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

            // Handle camera orientation request
            if let Some(orient_req) = ctx.camera_orient_request.take() {
                self.camera.orient_to_plane(
                    glam::Vec3::from_array(orient_req.plane_origin),
                    glam::Vec3::from_array(orient_req.plane_normal),
                    glam::Vec3::from_array(orient_req.plane_up),
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

    fn handle_select_tool(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                // Select the hovered body, or deselect if clicking empty space
                if let Some(hovered) = self.hovered_body {
                    if self.selected_body == Some(hovered) {
                        // Clicking on already selected body - deselect
                        self.selected_body = None;
                        app_log::info("Deselected body");
                    } else {
                        // Select the new body
                        self.selected_body = Some(hovered);
                        app_log::info(format!("Selected body: {hovered:?}"));
                    }
                } else {
                    // Clicked on empty space - deselect
                    if self.selected_body.is_some() {
                        self.selected_body = None;
                        app_log::info("Deselected (clicked empty space)");
                    }
                }
                true // Request redraw
            }
            _ => false,
        }
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
    }
}
