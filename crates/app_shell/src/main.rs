mod camera;
mod orientation_cube;
mod ui;

use anyhow::{Context, Result};
use camera::CameraController;
use core_document::{Document, DocumentService};
use glam::Vec3;
use kernel_api::TriMesh;
use orientation_cube::OrientationCubeInput;
use render_vk::{
    BodySubmission, FrameSubmission, GpuLight, HighlightState, LightingData, RenderBackend,
    RenderSettings, ViewportRect as RenderViewportRect, VulkanRenderer,
};
use settings::{LightingSettings, SettingsStore, UserSettings};
use std::time::{Duration, Instant};
use tracing::{error, info, warn};
use ui::{ActiveTool, UiLayer};
use uuid::Uuid;
use wb_part::PartDesignWorkbench;
use wb_sketch::SketchWorkbench;
use winit::{
    application::ApplicationHandler,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Window, WindowAttributes, WindowId},
};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let document = Document::new("Untitled");
    let mut registry = DocumentService::default();
    registry.register_workbench(Box::new(SketchWorkbench::default()))?;
    registry.register_workbench(Box::new(PartDesignWorkbench::default()))?;
    info!(
        "Registered {} workbenches",
        registry.workbench_descriptors().count()
    );
    info!("Loaded document `{}` ({})", document.name(), document.id());

    let settings_store = SettingsStore::new().context("settings store init failed")?;
    let user_settings = match settings_store.load() {
        Ok(settings) => settings,
        Err(err) => {
            warn!("using default settings: {err}");
            UserSettings::default()
        }
    };

    let event_loop = EventLoop::new().context("failed to create event loop")?;
    let mut render_settings = RenderSettings::default();
    render_settings.preferred_gpu = user_settings.preferred_gpu.clone();
    render_settings.msaa_samples = user_settings.rendering.msaa_samples;
    let mut app = PrintCadApp::new(render_settings, settings_store, user_settings);
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
    demo_bodies: Vec<BodySubmission>,
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
    // Selected body ID
    selected_body: Option<Uuid>,
    // Hovered body ID (for highlighting)
    hovered_body: Option<Uuid>,
    // Hovered world position (for status bar display)
    hovered_world_pos: Option<[f32; 3]>,
    // Current cursor position in viewport
    cursor_in_viewport: Option<(f32, f32)>,
}

impl PrintCadApp {
    fn new(
        settings: RenderSettings,
        settings_store: SettingsStore,
        user_settings: UserSettings,
    ) -> Self {
        let mut camera = CameraController::new(&user_settings.camera, (1, 1));
        let demo_bodies = demo_bodies();
        if let Some((center, radius)) = bodies_bounds(&demo_bodies) {
            camera.reset_to_fit(center, radius);
        }

        Self {
            settings,
            renderer: None,
            frame_submission: FrameSubmission::default(),
            window: None,
            window_id: None,
            ui_layer: None,
            demo_bodies,
            settings_store,
            user_settings,
            camera,
            active_tool: ActiveTool::Select,
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

        // Track cursor position for picking
        if let WindowEvent::CursorMoved { position, .. } = &event {
            // Store cursor position in window coordinates
            let x = position.x as u32;
            let y = position.y as u32;

            // Request GPU picking at cursor position
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.request_pick(x, y);
            }

            // Store cursor position relative to viewport for other uses
            let vp = self.camera.viewport_info();
            let cursor_x = position.x as f32 - vp.0;
            let cursor_y = position.y as f32 - vp.1;

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

        let (window, renderer) = match (self.window.as_ref(), self.renderer.as_mut()) {
            (Some(window), Some(renderer)) => (window, renderer),
            _ => return,
        };

        // Update camera animation
        self.camera.update(dt_secs);

        // Apply highlight states to bodies
        self.frame_submission.bodies = self
            .demo_bodies
            .iter()
            .map(|body| {
                let is_hovered = self.hovered_body == Some(body.id);
                let is_selected = self.selected_body == Some(body.id);
                let highlight = match (is_hovered, is_selected) {
                    (true, true) => HighlightState::HoveredAndSelected,
                    (true, false) => HighlightState::Hovered,
                    (false, true) => HighlightState::Selected,
                    (false, false) => HighlightState::None,
                };
                BodySubmission {
                    highlight,
                    ..body.clone()
                }
            })
            .collect();
        self.frame_submission.view_proj = self.camera.view_projection();
        self.frame_submission.camera_pos = self.camera.position();
        self.frame_submission.lighting = lighting_data_from_settings(&self.user_settings.lighting);

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
            );
            self.frame_submission.egui = Some(ui_result.submission);
            self.active_tool = ui_result.active_tool;

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
                    warn!("failed to save settings: {err}");
                }
            }
        } else {
            self.frame_submission.egui = None;
            self.frame_submission.viewport_rect = None;
        }

        window.request_redraw();

        if let Err(err) = renderer.render(&self.frame_submission) {
            error!("render failure: {err}");
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
    }
}

impl PrintCadApp {
    fn handle_tool_input(&mut self, event: &WindowEvent) -> bool {
        match self.active_tool {
            ActiveTool::Select => self.handle_select_tool(event),
            ActiveTool::SketchLine | ActiveTool::SketchCircle => self.handle_sketch_tool(event),
            ActiveTool::Pad | ActiveTool::Pocket => self.handle_part_tool(event),
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
                        info!("Deselected body");
                    } else {
                        // Select the new body
                        self.selected_body = Some(hovered);
                        info!("Selected body: {:?}", hovered);
                    }
                } else {
                    // Clicked on empty space - deselect
                    if self.selected_body.is_some() {
                        self.selected_body = None;
                        info!("Deselected (clicked empty space)");
                    }
                }
                true // Request redraw
            }
            _ => false,
        }
    }

    fn handle_sketch_tool(&mut self, event: &WindowEvent) -> bool {
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
            ..
        } = event
        {
            info!("Sketch tool click captured");
            return true;
        }
        false
    }

    fn handle_part_tool(&mut self, event: &WindowEvent) -> bool {
        if let WindowEvent::MouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
            ..
        } = event
        {
            info!("Part tool click captured");
            return true;
        }
        false
    }
}

fn demo_bodies() -> Vec<BodySubmission> {
    vec![BodySubmission {
        id: Uuid::new_v4(),
        mesh: pyramid_mesh(),
        color: [0.85, 0.55, 0.3],
        highlight: HighlightState::None,
    }]
}

fn bodies_bounds(bodies: &[BodySubmission]) -> Option<(Vec3, f32)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;

    for body in bodies {
        for pos in &body.mesh.positions {
            let v = Vec3::from_array(*pos);
            min = min.min(v);
            max = max.max(v);
            any = true;
        }
    }

    if !any {
        return None;
    }

    let center = (min + max) * 0.5;
    let mut radius = 0.0f32;
    for body in bodies {
        for pos in &body.mesh.positions {
            let v = Vec3::from_array(*pos);
            radius = radius.max((v - center).length());
        }
    }

    Some((center, radius))
}

fn pyramid_mesh() -> TriMesh {
    // Pyramid with base on XY plane, apex pointing up (+Z)
    let bl = [-0.4, -0.4, 0.0];
    let br = [0.4, -0.4, 0.0];
    let tr = [0.4, 0.4, 0.0];
    let tl = [-0.4, 0.4, 0.0];
    let apex = [0.0, 0.0, 0.7];

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();

    // Side faces: CCW winding when viewed from OUTSIDE (normal points outward)
    // Front face (normal points toward -Y)
    add_triangle(&mut positions, &mut normals, &mut indices, [br, apex, bl]);
    // Right face (normal points toward +X)
    add_triangle(&mut positions, &mut normals, &mut indices, [tr, apex, br]);
    // Back face (normal points toward +Y)
    add_triangle(&mut positions, &mut normals, &mut indices, [tl, apex, tr]);
    // Left face (normal points toward -X)
    add_triangle(&mut positions, &mut normals, &mut indices, [bl, apex, tl]);
    // Bottom face (normal points toward -Z): CCW when viewed from below
    add_triangle(&mut positions, &mut normals, &mut indices, [tr, br, bl]);
    add_triangle(&mut positions, &mut normals, &mut indices, [tl, tr, bl]);

    TriMesh {
        positions,
        normals,
        indices,
    }
}

fn add_triangle(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    indices: &mut Vec<u32>,
    verts: [[f32; 3]; 3],
) {
    let normal = face_normal(verts[0], verts[1], verts[2]);
    let start = positions.len() as u32;
    for v in verts {
        positions.push(v);
        normals.push(normal);
    }
    indices.extend_from_slice(&[start, start + 1, start + 2]);
}

fn face_normal(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> [f32; 3] {
    let pa = Vec3::from_array(a);
    let pb = Vec3::from_array(b);
    let pc = Vec3::from_array(c);
    let normal = (pb - pa).cross(pc - pa).normalize_or_zero();
    normal.to_array()
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
