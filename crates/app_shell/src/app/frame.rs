//! Per-frame work: pacing, scene submission assembly, UI run, render, pick.

use std::sync::Arc;
use std::time::{Duration, Instant};

use core_document::WorkbenchFeature;
use glam::Vec3;
use render_vk::{
    BodySubmission, GpuLight, HighlightState, LightingData, RenderBackend,
    ViewportRect as RenderViewportRect,
};
use settings::{ProjectionMode, SixDofButtonAction, UserSettings};
use uuid::Uuid;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use crate::log_panel as app_log;
use crate::orientation_cube::{CameraSnapView, OrientationCubeInput};
use crate::{Document, PrintCadApp, ui};

/// Stable u64 fingerprint of a [`kernel_api::TriMesh`]'s geometry. Used as
/// the `revision` for workbench overlay meshes so unchanged overlays are
/// cache hits instead of per-frame re-uploads. Overlay meshes are tiny
/// (grid lines, guides), so hashing them is cheap.
fn hash_trimesh(mesh: &kernel_api::TriMesh) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for p in &mesh.positions {
        for v in p {
            v.to_bits().hash(&mut hasher);
        }
    }
    for c in &mesh.colors {
        for v in c {
            v.to_bits().hash(&mut hasher);
        }
    }
    mesh.indices.hash(&mut hasher);
    mesh.edges.hash(&mut hasher);
    hasher.finish()
}

/// Stable u64 fingerprint of a serde JSON value. Used as a `revision`
/// counter for sketch geometry so the GPU mesh cache can skip the upload
/// when the underlying sketch JSON hasn't changed between frames.
pub(crate) fn hash_revision(value: &serde_json::Value) -> u64 {
    use std::hash::{Hash, Hasher};
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Union of all imported mesh AABBs in world space `(min, max)`.
pub(crate) fn document_imported_aabb(document: &Document) -> Option<(Vec3, Vec3)> {
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

pub(crate) fn aabb_fit_center_radius(aabb_min: Vec3, aabb_max: Vec3) -> (Vec3, f32) {
    let center = (aabb_min + aabb_max) * 0.5;
    let extents = aabb_max - aabb_min;
    let radius = extents.length() * 0.5;
    (center, radius.max(1.0))
}

pub(crate) fn lighting_data_from_settings(user: &UserSettings) -> LightingData {
    let settings = &user.lighting;
    let preset = user.camera.axis_preset;
    LightingData {
        main_light: GpuLight::new(
            settings.main_light.direction_world(preset),
            settings.main_light.color,
            settings.main_light.intensity,
            settings.main_light.enabled,
        ),
        backlight: GpuLight::new(
            settings.backlight.direction_world(preset),
            settings.backlight.color,
            settings.backlight.intensity,
            settings.backlight.enabled,
        ),
        fill_light: GpuLight::new(
            settings.fill_light.direction_world(preset),
            settings.fill_light.color,
            settings.fill_light.intensity,
            settings.fill_light.enabled,
        ),
        ambient_color: settings.ambient_color,
        ambient_intensity: settings.ambient_intensity,
        specular_shininess: settings.specular_shininess,
        specular_intensity: settings.specular_intensity,
        edge_line_color: settings.edge_line_color,
        edge_line_width: settings.edge_line_width,
    }
}

/// The command a button action asks for. Switching projection needs to know
/// which one is in force, since it names the one to change to.
fn device_button_command(
    action: SixDofButtonAction,
    projection: ProjectionMode,
) -> Option<ui::UiCommand> {
    let snap = |view| Some(ui::UiCommand::CameraSnap(view));
    match action {
        SixDofButtonAction::None => None,
        SixDofButtonAction::FitView => Some(ui::UiCommand::FitView),
        SixDofButtonAction::ViewIsometric => snap(CameraSnapView::FrontTopRight),
        SixDofButtonAction::ViewFront => snap(CameraSnapView::Front),
        SixDofButtonAction::ViewRear => snap(CameraSnapView::Rear),
        SixDofButtonAction::ViewLeft => snap(CameraSnapView::Left),
        SixDofButtonAction::ViewRight => snap(CameraSnapView::Right),
        SixDofButtonAction::ViewTop => snap(CameraSnapView::Top),
        SixDofButtonAction::ViewBottom => snap(CameraSnapView::Bottom),
        SixDofButtonAction::ToggleProjection => {
            Some(ui::UiCommand::SetProjection(match projection {
                ProjectionMode::Perspective => ProjectionMode::Orthographic,
                ProjectionMode::Orthographic => ProjectionMode::Perspective,
            }))
        }
    }
}

impl PrintCadApp {
    /// Body of `about_to_wait`: pace the frame, drain worker channels,
    /// assemble the scene, run the UI, render, read back the pick, and
    /// apply this frame's UI commands.
    /// Every background source that must keep frames coming until it
    /// settles. The wake gate and the end-of-frame scheduler share this —
    /// a source listed in only one of them either burns CPU or sleeps
    /// through its own completion.
    fn async_work_pending(&self) -> bool {
        self.kernel_worker.in_flight() > 0
            || self.server.status().busy()
            || self.file_dialog_rx.is_some()
            || self.document_save_rx.is_some()
            || self.document_open_rx.is_some()
            || self.step_import_pending.is_some()
            || !self.nav_device.motion().is_idle()
    }

    /// What the 6-DoF mouse's buttons ask for, as commands. The device
    /// reports a press and a release; the press is the one that acts.
    /// Its motion is read in [`Self::build_scene_submission`], where the
    /// camera is.
    fn device_button_commands(&mut self) -> Vec<ui::UiCommand> {
        let mut commands = Vec::new();
        for button in self.nav_device.take_buttons() {
            tracing::debug!(
                target: "printcad.input",
                index = button.index,
                pressed = button.pressed,
                "6-DoF mouse button"
            );
            if !button.pressed {
                continue;
            }
            let action = self
                .user_settings
                .sixdof
                .buttons
                .get(button.index as usize)
                .copied()
                .unwrap_or_default();
            if let Some(command) =
                device_button_command(action, self.user_settings.camera.projection)
            {
                commands.push(command);
            }
        }
        commands
    }

    pub(crate) fn frame(&mut self, event_loop: &ActiveEventLoop) {
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
        }
        // Uncapped: no control-flow decision here. The end of the frame
        // schedules the next one only if something needs it (render on
        // demand); an idle scene sleeps until the next event.

        // `about_to_wait` runs on every loop wake, including compositor
        // frame callbacks acknowledging our own presents. Render only when
        // someone asked (scheduler, input, OS expose) or state demands it —
        // otherwise presenting would wake us again and the loop never rests.
        {
            let input_active = self
                .last_input_time
                .is_some_and(|t| t.elapsed() < Duration::from_millis(150));
            let animating = self.camera.is_animating()
                || std::env::var_os("PRINTCAD_BENCH_ORBIT").is_some()
                || std::env::var_os("PRINTCAD_EXIT_AFTER_MS").is_some()
                || std::env::var_os("PRINTCAD_BENCH_SPIN").is_some();
            if !(self.redraw_needed
                || input_active
                || self.async_work_pending()
                || animating
                || self.pending_ui_repaint.is_zero())
            {
                event_loop.set_control_flow(ControlFlow::Wait);
                return;
            }
        }
        self.redraw_needed = false;

        // Time since last *rendered* frame
        let dt_secs = if let Some(last) = self.last_frame_time {
            let elapsed = now - last;
            let dt = elapsed.as_secs_f32();

            // Waking from an on-demand sleep: the gap is idle time, not a
            // slow frame. Drop the seed so the next dt restarts the average;
            // the displayed number stays live per frame below.
            if dt > 0.5 {
                self.fps_accum_time = 0.0;
                self.fps_frame_count = 0;
                self.smoothed_frame_s = None;
            } else if dt > 0.0 {
                // The display updates every frame from a smoothed frame time
                // — a real number one frame after waking, without the jitter
                // of raw per-frame values. The 1 s accumulator below only
                // paces the log line.
                let sft = match self.smoothed_frame_s {
                    None => dt,
                    Some(prev) => prev * 0.85 + dt * 0.15,
                };
                self.smoothed_frame_s = Some(sft);
                self.current_fps = 1.0 / sft.max(1e-6);

                self.fps_accum_time += dt;
                self.fps_frame_count += 1;
                if self.fps_accum_time >= 1.0 {
                    self.fps_accum_time = 0.0;
                    self.fps_frame_count = 0;
                    let (ui_ms, render_ms, frames) = self.frame_phase_accum;
                    if frames > 0 {
                        let stats = self
                            .gfx
                            .as_ref()
                            .map(|g| g.renderer.last_draw_stats())
                            .unwrap_or_default();
                        tracing::info!(
                            target: "printcad.frame",
                            fps = self.current_fps,
                            ui_ms = ui_ms / frames as f32,
                            render_ms = render_ms / frames as f32,
                            bodies = self.frame_submission.bodies.len(),
                            drawn = stats.bodies_drawn,
                            culled = stats.bodies_culled,
                            tri_idx = stats.triangle_indices,
                            edge_idx = stats.edge_indices,
                            scene_redraws = self.scene_redraw_accum,
                            status_changes = self.status_changes_accum,
                            wake = ?self.last_wake_reason,
                            "frame phases (1s avg)"
                        );
                    }
                    self.frame_phase_accum = (0.0, 0.0, 0);
                    self.scene_redraws_per_s = self.scene_redraw_accum;
                    self.scene_redraw_accum = 0;
                    self.status_changes_accum = 0;
                }
            }
            dt
        } else {
            0.016 // ~60fps default for first frame
        };

        self.last_frame_time = Some(now);

        // Dev/bench hook: orbit continuously so frame measurements cover the
        // moving-camera case (the one the user feels).
        if std::env::var_os("PRINTCAD_BENCH_ORBIT").is_some() {
            self.camera.apply_rotate_delta(
                &crate::orientation_cube::RotateDelta {
                    axis: crate::orientation_cube::RotateAxis::ScreenY,
                    degrees: 0.5,
                },
                &self.user_settings.camera,
            );
        }

        // Dev/bench hook: quit through the real exit path after a delay, so
        // teardown can be timed on a loaded document without a dialog.
        if let Some(after_ms) = std::env::var("PRINTCAD_EXIT_AFTER_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            && self.bench_started.elapsed().as_millis() as u64 >= after_ms
        {
            tracing::info!(target: "printcad.frame", "bench exit requested");
            self.wait_for_document_saves();
            event_loop.exit();
            return;
        }

        // Dev/bench hooks: import a STEP or open a document at startup
        // without a dialog.
        if !self.bench_open_fired {
            self.bench_open_fired = true;
            if let Ok(path) = std::env::var("PRINTCAD_OPEN_FILE") {
                let detail = self.last_step_import_detail.clone();
                self.import_step_at(std::path::Path::new(&path), detail);
            }
            if let Ok(path) = std::env::var("PRINTCAD_OPEN_DOC") {
                self.open_document_at(std::path::PathBuf::from(path));
            }
            if std::env::var_os("PRINTCAD_BENCH_SKETCH").is_some() {
                self.bench_open_sketch();
            }
        }
        // Dev/bench hook: `PRINTCAD_BENCH_SELECT=<n or name>` selects the
        // n-th body, or the first whose name contains the text, once it has
        // geometry, so a capture can show the selection overlay without a
        // click.
        if !self.bench_select_fired
            && let Ok(which) = std::env::var("PRINTCAD_BENCH_SELECT")
            && let Some(body) = match which.parse::<usize>() {
                Ok(n) => self.document.bodies().get(n),
                Err(_) => self
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.name.contains(&which)),
            }
            .map(|b| (b.id, b.name.clone()))
            && self.document.imported_geometry(body.0).is_some()
        {
            self.bench_select_fired = true;
            let row = match self.document.imported_object_for_body(body.0) {
                Some(node) => crate::ui::TreeItemId::ImportedObject(node),
                None => crate::ui::TreeItemId::Body(body.0),
            };
            self.apply_tree_selection(row);
            tracing::info!(target: "printcad.frame", "bench selected body {:?} `{}`", body.0, body.1);
        }

        let mut new_body_requested = false;
        let server_status = self.server.status();
        let ui_repaint_delay;

        // Pull any STEP imports that the kernel worker finished off the
        // queue before we build this frame's submission, so freshly imported
        // bodies show up immediately and the import log lines stay tied to
        // the frame they actually became visible in. Has to happen before
        // we take a mutable borrow on `self.renderer` below.
        self.drain_kernel_responses();
        self.drain_document_saves();
        self.drain_server_messages();
        self.drain_document_opens();
        self.drive_part_recompute();

        if self.gfx.is_none() {
            return;
        }

        // Update camera animation and assemble this frame's scene submission
        // before the UI/render block takes its borrows on `gfx`.
        self.call_workbench_on_frame(dt_secs);
        let viewport_data = self.build_scene_submission(dt_secs);
        if self.screen == crate::ui::Screen::Start {
            self.frame_submission.bodies.clear();
        }
        let ViewportData {
            overlays: screen_space_overlays,
            marks: screen_space_marks,
            labels: screen_space_labels,
            hud: viewport_hud,
            status: status_items,
            task,
            editing_feature,
        } = viewport_data;
        let planar_view_lock = self.sketch_editing_active();
        let hover_card = self.hover_card();
        let dimensions = self.selection_dimensions();
        let host_params = ui::HostCtxParams {
            camera_position: self.camera.position(),
            camera_target: self.camera.target(),
            viewport: self
                .frame_submission
                .viewport_rect
                .map(|r| (r.x, r.y, r.width, r.height))
                .unwrap_or((0, 0, 1, 1)),
            view_proj: Some(self.camera.view_projection()),
            selected_body_id: self.active_body_id.map(|id| id.0),
            selected_face: self.last_face_hit.as_ref().map(|(_, f)| *f),
        };

        let commands;

        {
            let Some(gfx) = self.gfx.as_mut() else {
                return;
            };
            let crate::app::Gfx {
                renderer,
                ui_layer,
                window,
                ..
            } = gfx;

            {
                let orientation_input = OrientationCubeInput {
                    camera_orientation: self.camera.orientation(),
                    axis_system: self.camera.axis_system(),
                };

                let pivot_screen_pos = self
                    .camera
                    .rotation_pivot_indicator_screen_px(self.user_settings.camera.orbit_pivot_pick);

                let ui_started = Instant::now();
                let ui_result = ui_layer.run(
                    window,
                    ui::UiFrameInputs {
                        screen: self.screen,
                        recent: &self.recent.files,
                        active_tool: self.active_tool.clone(),
                        active_workbench: self.active_workbench.clone(),
                        settings: &self.user_settings,
                        document: &mut self.document,
                        registry: &mut self.registry,
                        host: host_params,
                        orientation_input: Some(&orientation_input),
                        planar_view_lock,
                        fps: (!self.fps_display_idle).then_some(self.current_fps),
                        scene_redraws_per_s: self.scene_redraws_per_s,
                        gpu_name: self.gpu_name.as_deref(),
                        gpus: &self.available_gpus,
                        hovered_point: self.hovered_world_pos,
                        pivot_screen_pos,
                        axis_system: self.camera.axis_system(),
                        tree_selection: self.tree_selection,
                        active_document_object: self.active_document_object,
                        editing_feature,
                        viewport_hud,
                        status_items,
                        task,
                        hover_card,
                        dimensions,
                        screen_space_overlays: &screen_space_overlays,
                        screen_space_marks: &screen_space_marks,
                        screen_space_labels: &screen_space_labels,
                        pending_imports: self.kernel_worker.in_flight(),
                        pending_document_open: server_status.opens_in_flight
                            + server_status.saves_in_flight
                            + u32::from(self.document_open_rx.is_some()),
                        kernel_status: {
                            let status = self.kernel_worker.status();
                            if status != self.last_status_text {
                                self.status_changes_accum += 1;
                                self.last_status_text = status.clone();
                            }
                            status
                        },
                        kernel_progress: self.kernel_worker.progress(),
                        kernel_cancellable: self.kernel_worker.is_cancellable(),
                        // Field reads, not `&self` methods: `gfx` is borrowed
                        // for the whole block.
                        document_saving: self.document_save_rx.is_some()
                            || server_status.saves_in_flight > 0,
                        save_progress: self.save_progress.as_ref().and_then(|p| p.read()),
                        server_label: match (server_status.connected, server_status.peers) {
                            (false, _) => format!("{} (disconnected)", self.server.name()),
                            (true, 0) => self.server.name().to_string(),
                            (true, 1) => format!("{} · 1 peer", self.server.name()),
                            (true, n) => format!("{} · {n} peers", self.server.name()),
                        },
                        reveal_body: self.reveal_body.take(),
                        viewport_menu: self.viewport_menu.clone(),
                        nav_device: self.nav_device.device_name(),
                        nav_buttons: self.nav_device.button_count(),
                        step_import_pending: self.step_import_pending.as_mut(),
                    },
                );
                self.frame_phase_accum.0 += ui_started.elapsed().as_secs_f32() * 1000.0;
                ui_repaint_delay = ui_result.repaint_delay;
                self.frame_submission.egui = Some(ui_result.submission);
                self.active_tool = ui_result.active_tool;
                self.active_workbench = ui_result.active_workbench;
                self.task_open = ui_result.task_open;

                // The window title follows the document and its dirty state.
                let title = if self.screen == crate::ui::Screen::Start {
                    "printCAD".to_string()
                } else if self.document.metadata().dirty() {
                    format!("{} • — printCAD", self.document.name())
                } else {
                    format!("{} — printCAD", self.document.name())
                };
                if title != self.window_title {
                    window.set_title(&title);
                    self.window_title = title;
                }

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

                // The Part Design workbench exposes "New Body" as an Action tool.
                // Action tools live in `active_ids` for exactly one frame; we
                // detect a fresh click here and consume it so the body-creation
                // call (deferred until after the renderer borrow ends) only
                // fires once.
                if self.active_tool.active_ids.remove("part.new_body") {
                    new_body_requested = true;
                }

                commands = ui_result.commands;
            }

            let render_started = Instant::now();
            if let Err(err) = renderer.render(&mut self.frame_submission) {
                app_log::error(format!("Render failure: {err}"));
                event_loop.exit();
                return;
            }
            self.frame_phase_accum.1 += render_started.elapsed().as_secs_f32() * 1000.0;
            self.frame_phase_accum.2 += 1;
            if renderer.scene_redrawn_last_frame() {
                self.scene_redraw_accum += 1;
            }

            // Render on demand: another frame is scheduled only while
            // something is moving, pending, or animating. A short tail after
            // input lets egui reactions and pick readbacks land; a frame
            // drawn without edges gets one more to restore them; egui's own
            // timed repaints (caret blink) become a timed wake-up. Otherwise
            // the loop sleeps until the next OS event.
            const INPUT_TAIL: Duration = Duration::from_millis(150);
            let input_active = self
                .last_input_time
                .is_some_and(|t| t.elapsed() < INPUT_TAIL);
            // Field-level reads, not `async_work_pending()`: a `&self`
            // method call cannot coexist with the live `gfx` borrow, and
            // this must be frame-END truth — a kernel job submitted during
            // this frame has to keep the loop awake. Mirror the helper.
            let work_pending = self.kernel_worker.in_flight() > 0
                || self.server.status().busy()
                || self.file_dialog_rx.is_some()
                || self.document_save_rx.is_some()
                || self.document_open_rx.is_some()
                || self.step_import_pending.is_some()
                || !self.nav_device.motion().is_idle();
            let animating = self.camera.is_animating()
                || std::env::var_os("PRINTCAD_BENCH_ORBIT").is_some()
                || std::env::var_os("PRINTCAD_EXIT_AFTER_MS").is_some()
                || std::env::var_os("PRINTCAD_BENCH_SPIN").is_some();
            self.pending_ui_repaint = ui_repaint_delay;
            self.last_wake_reason = (
                input_active,
                work_pending,
                animating,
                ui_repaint_delay.is_zero(),
            );
            if input_active || work_pending || animating || ui_repaint_delay.is_zero() {
                self.fps_display_idle = false;
                self.redraw_needed = true;
                window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            } else if !self.fps_display_idle {
                // Going idle: paint one closing frame whose FPS display says
                // so, then sleep for real.
                self.fps_display_idle = true;
                self.redraw_needed = true;
                window.request_redraw();
                event_loop.set_control_flow(ControlFlow::Wait);
            } else if ui_repaint_delay < Duration::from_secs(10) {
                // The timed wake (caret blink etc.) should render once.
                self.redraw_needed = true;
                event_loop
                    .set_control_flow(ControlFlow::WaitUntil(Instant::now() + ui_repaint_delay));
            } else if fps_cap <= 0.0 {
                event_loop.set_control_flow(ControlFlow::Wait);
            }

            // Retrieve pick result from GPU picking (processed during render)
            let pick_result = renderer.latest_pick_result();
            self.hovered_body = pick_result.body_id;
            self.hovered_world_pos = pick_result.world_position;
        }

        // Apply this frame's UI actions now that the renderer borrow is over.
        let mut commands = commands;
        commands.extend(self.device_button_commands());
        self.apply_ui_commands(commands, new_body_requested, event_loop);
        // A tool clicked in the toolbar acts in the same frame.
        self.dispatch_activated_tools();

        self.publish_presence();

        // Everything this frame edited is in the outbox; hand it to the
        // server. One send per frame keeps drags coalesced (the buffer
        // collapsed them) and the wire quiet when nothing changed.
        let ops = self.document.take_pending_ops();
        if !ops.is_empty() {
            self.server
                .send(core_document::server::ClientMessage::Ops(ops));
        }

        // Cut an undo boundary at frame end when no drag is in progress so
        // an entire drag interaction coalesces into one step.
        // A task panel's edits stay one gesture until it closes.
        if self.mouse_buttons_down == 0 && !self.task_open {
            self.journal.note(&mut self.document);
        }
    }

    /// Update the camera and assemble this frame's [`FrameSubmission`]
    /// (sketch tessellations, imported bodies, workbench overlay meshes).
    /// Returns the workbench's screen-space overlays, which are drawn via
    /// egui rather than the 3D pass.
    /// True while the active workbench has an edit session open that keeps
    /// the view square to its plane. Gates the camera's out-of-plane
    /// rotation and the hover feedback that would compete with the bench's
    /// own.
    pub(crate) fn sketch_editing_active(&self) -> bool {
        self.registry
            .workbench(&self.active_workbench.0)
            .is_ok_and(|wb| wb.locks_view_to_plane() && wb.editing_feature().is_some())
    }

    fn build_scene_submission(&mut self, dt_secs: f32) -> ViewportData {
        self.camera.set_orbit_lock(self.sketch_editing_active());
        self.camera.flush_pending_wheel(&self.user_settings.camera);
        // Before the clip planes, so they are computed for the pose this
        // frame actually shows.
        let device_motion = self.nav_device.motion();
        self.camera.apply_device_motion(
            device_motion.axis_readings(),
            dt_secs,
            &self.user_settings.camera,
            &self.user_settings.sixdof,
        );
        self.camera
            .apply_auto_clip_planes(&self.user_settings.camera);
        self.camera.update(dt_secs, &self.user_settings.camera);

        // Collect sketch features from document and convert to meshes.
        //
        // Sketch geometry is recomputed every frame (it's cheap), but we
        // bump a per-feature revision based on the underlying JSON so the
        // renderer's cache only re-uploads when the sketch actually changes.
        // The sketch currently being edited is drawn as crisp screen-space
        // overlays by the workbench; only sketches NOT under edit get the 3D
        // tessellation (drawing both would double-render the active one).
        let editing_sketch = self
            .registry
            .workbench(&self.active_workbench.0)
            .ok()
            .and_then(|wb| wb.editing_feature());
        let sketch_meshes: Vec<BodySubmission> = self
            .document
            .feature_tree()
            .all_nodes()
            .filter_map(|(feature_id, node)| {
                if node.workbench_id.as_str() != "wb.sketch" {
                    return None;
                }
                if Some(*feature_id) == editing_sketch || !node.visible {
                    return None;
                }

                let sketch_feature = wb_sketch::SketchFeature::from_json(&node.data).ok()?;

                let mesh = wb_sketch::render::sketch_to_lines(
                    &sketch_feature.sketch,
                    &sketch_feature.plane,
                );

                // Hash the serialized sketch JSON for a stable revision: the
                // renderer skips the upload when the sketch is unchanged.
                let revision = hash_revision(&node.data);

                // Match the in-edit overlay palette: white geometry,
                // orange hover, green selection. Color is baked directly so
                // the tint is unmistakable even on hairline geometry.
                let is_selected = self.active_document_object == Some(*feature_id)
                    || self.tree_selection == Some(crate::ui::TreeItemId::Feature(*feature_id));
                let is_hovered = self.hovered_sketch == Some(*feature_id);
                let color = if is_selected {
                    [0.35, 0.95, 0.45]
                } else if is_hovered {
                    [1.0, 0.75, 0.2]
                } else {
                    [0.85, 0.85, 0.85]
                };
                // The tint participates in the cache revision so hover /
                // selection transitions actually re-upload the color.
                let state_bits = (is_selected as u64) | ((is_hovered as u64) << 1);
                Some(BodySubmission {
                    id: feature_id.0,
                    revision: revision ^ (state_bits << 62),
                    mesh: Arc::new(mesh),
                    color,
                    highlight: HighlightState::None,
                    is_wireframe: false,
                    opacity: 1.0,
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
            .filter(|(body_id, _)| self.document.imported_body_effective_visible(**body_id))
            .map(|(body_id, geometry)| {
                // Selection is painted by the overlay below, never by a
                // tint; hover brightens, and a peer's selection tints
                // subordinate to it, so your own interaction always wins.
                let is_hovered = self.hovered_body == Some(body_id.0);
                let is_peer_selected = self
                    .peer_presence
                    .values()
                    .any(|p| p.selected_body == Some(body_id.0));
                let highlight = if is_hovered {
                    HighlightState::Hovered
                } else if is_peer_selected {
                    HighlightState::PeerSelected
                } else {
                    HighlightState::None
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
                    opacity: 1.0,
                    highlight,
                    is_wireframe: false,
                }
            })
            .collect();

        let wb_id = self.active_workbench.0.clone();

        // Overlay meshes from the active workbench (grid lines, guides, etc.)
        let params = self.overlay_ctx_params();
        let mut overlay_meshes: Vec<BodySubmission> = self
            .with_workbench_ctx(&wb_id, params, |wb, ctx| {
                wb.get_overlay_meshes(ctx, ctx.active_document_object)
            })
            .map(|(meshes, _outcome)| meshes)
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, (mesh, color, is_wireframe))| {
                // Overlays are regenerated every frame; give slot `i` a
                // stable id from the pool and a content-hash revision so an
                // unchanged overlay is a GPU cache hit instead of a
                // guaranteed upload + GC every frame.
                while self.overlay_id_pool.len() <= i {
                    self.overlay_id_pool.push(Uuid::new_v4());
                }
                BodySubmission {
                    id: self.overlay_id_pool[i],
                    revision: hash_trimesh(&mesh),
                    mesh: Arc::new(mesh),
                    color,
                    opacity: 1.0,
                    highlight: HighlightState::None,
                    is_wireframe,
                }
            })
            .collect();

        // Screen-space overlays + labels from the active workbench
        // (constant-thickness lines and constant-size text).
        let params = self.overlay_ctx_params();
        let mut data = self
            .with_workbench_ctx(&wb_id, params, |wb, ctx| ViewportData {
                overlays: wb.get_screen_space_overlays(ctx, ctx.active_document_object),
                marks: wb.get_screen_space_marks(ctx, ctx.active_document_object),
                labels: wb.get_screen_space_labels(ctx, ctx.active_document_object),
                hud: wb.viewport_hud(ctx),
                status: wb.status_items(ctx),
                task: wb.task(ctx),
                editing_feature: wb.editing_feature(),
            })
            .map(|(data, _outcome)| data)
            .unwrap_or_default();
        let screen_space_labels = &mut data.labels;

        // Peers' cursors: a named marker where each other editor points.
        // Same projection the overlays use; a cursor behind the camera or
        // outside the model simply has no marker this frame.
        for state in self.peer_presence.values() {
            let Some(world) = state.cursor_world else {
                continue;
            };
            let Some((x, y)) = self
                .camera
                .world_to_screen(Vec3::new(world[0], world[1], world[2]))
            else {
                continue;
            };
            screen_space_labels.push(
                core_document::ScreenSpaceLabel::new(
                    [x, y - 14.0],
                    format!("⯆ {}", state.display_name),
                    [0.35, 0.75, 0.95],
                    12.0,
                )
                .pill(),
            );
        }

        // Combine sketch meshes, imported geometry, and overlay meshes.
        let mut all_meshes = sketch_meshes;
        all_meshes.extend(imported_meshes);
        all_meshes.append(&mut overlay_meshes);

        // The selection overlay, in the selection paint at the chosen
        // opacity: a selected face is its own triangles, slightly lifted off
        // the surface; a selected body is the whole body's mesh again,
        // drawn over itself.
        let paint = self.user_settings.rendering.selection_color;
        let opacity = self
            .user_settings
            .rendering
            .selection_opacity
            .min(settings::MAX_SELECTION_OPACITY);
        if let Some(face) = &self.face_highlight {
            all_meshes.push(BodySubmission {
                id: self.face_highlight_id,
                revision: face.revision,
                mesh: Arc::clone(&face.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
            });
        } else if let Some(geometry) = self
            .selected_body
            .and_then(|id| self.document.imported_geometry(core_document::BodyId(id)))
        {
            // One slot serves every body; the body's id in the revision
            // keeps two bodies at the same revision from sharing buffers.
            let (hi, lo) = self.selected_body.unwrap_or_default().as_u64_pair();
            all_meshes.push(BodySubmission {
                id: self.body_highlight_id,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
            });
        }

        self.frame_submission.bodies = all_meshes;
        self.frame_submission.view_proj = self.camera.view_projection();
        self.frame_submission.camera_pos = self.camera.position();
        self.frame_submission.lighting = lighting_data_from_settings(&self.user_settings);

        data
    }
}

/// Everything a workbench contributes to the viewport and chrome in one
/// frame, gathered under a single fully populated runtime context.
#[derive(Default)]
pub(crate) struct ViewportData {
    pub overlays: Vec<core_document::ScreenSpaceOverlay>,
    pub marks: Vec<core_document::ScreenSpaceMark>,
    pub labels: Vec<core_document::ScreenSpaceLabel>,
    pub hud: Option<core_document::ViewportHud>,
    pub status: Option<core_document::StatusItems>,
    pub task: Option<core_document::TaskInfo>,
    pub editing_feature: Option<core_document::FeatureId>,
}

impl PrintCadApp {
    /// Dev/bench hook: a body with a small constrained sketch, opened for
    /// editing, so the sketcher can be exercised without clicking.
    fn bench_open_sketch(&mut self) {
        use wb_sketch::sketch::{
            Circle, Constraint, ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D,
        };
        self.create_new_body();
        let body = self.active_body_id;
        let mut sketch = Sketch::new("Sketch");
        let p = |s: &mut Sketch, x: f32, y: f32| {
            s.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(x, y))))
        };
        let a = p(&mut sketch, 0.0, 0.0);
        let b = p(&mut sketch, 80.0, 0.0);
        let c = p(&mut sketch, 80.0, 24.0);
        let d = p(&mut sketch, 0.0, 24.0);
        let bottom = sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
        let right = sketch.add_geometry(GeometryElement::Line(Line::new(b, c)));
        sketch.add_geometry(GeometryElement::Line(Line::new(c, d)));
        sketch.add_geometry(GeometryElement::Line(Line::new(d, a)));
        let center = p(&mut sketch, 22.0, 12.0);
        let circle = sketch.add_geometry(GeometryElement::Circle(Circle::new(center, 7.2)));
        for kind in [
            ConstraintKind::Horizontal { element: bottom },
            ConstraintKind::Vertical { element: right },
            ConstraintKind::Length {
                line: bottom,
                length: 80.0,
            },
            ConstraintKind::Length {
                line: right,
                length: 24.0,
            },
            ConstraintKind::Diameter {
                circle,
                diameter: 14.4,
            },
        ] {
            sketch.constraints.push(Constraint::new(kind));
        }
        let plane = sketch.plane;
        let sketch_id = match self.document.add_feature_in_body(
            wb_sketch::SketchFeature::new(sketch, plane),
            "Sketch".into(),
            body,
        ) {
            Ok(id) => id,
            Err(err) => {
                app_log::error(format!("bench sketch: {err}"));
                return;
            }
        };
        // `pad` pads the sketch and opens the pad's task; anything else
        // opens the sketch for editing.
        let pad = std::env::var("PRINTCAD_BENCH_SKETCH").is_ok_and(|v| v == "pad" || v == "pocket");
        if !pad {
            self.apply_tree_activation(crate::ui::TreeItemId::Feature(sketch_id));
            return;
        }
        let pad = wb_part::PartFeature::Pad {
            sketch: sketch_id,
            length: 20.0,
            reversed: false,
            symmetric: false,
            mode: wb_part::ExtrudeMode::Dimension,
            length2: 0.0,
            taper_deg: 0.0,
            up_to_face: None,
            up_to_offset: 0.0,
        };
        let pad_id = match self.document.add_feature_in_body(pad, "Pad".into(), body) {
            Ok(id) => {
                self.document.mark_feature_dirty(id);
                self.document.set_feature_visible(sketch_id, false);
                self.apply_tree_selection(crate::ui::TreeItemId::Feature(id));
                id
            }
            Err(err) => {
                app_log::error(format!("bench pad: {err}"));
                return;
            }
        };
        // `pocket` adds a face sketch on the pad's top and pockets it.
        if std::env::var("PRINTCAD_BENCH_SKETCH").is_ok_and(|v| v == "pocket") {
            let mut top = Sketch::new("sketch_1");
            top.plane =
                wb_sketch::sketch::SketchPlane::from_face([0.0, 0.0, 20.0], [0.0, 0.0, 1.0]);
            let center =
                top.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(40.0, 12.0))));
            top.add_geometry(GeometryElement::Circle(Circle::new(center, 5.0)));
            let plane = top.plane;
            let top_id = match self.document.add_feature_in_body(
                wb_sketch::SketchFeature::new(top, plane),
                "sketch_1".into(),
                body,
            ) {
                Ok(id) => id,
                Err(err) => {
                    app_log::error(format!("bench face sketch: {err}"));
                    return;
                }
            };
            let pocket = wb_part::PartFeature::Pocket {
                sketch: top_id,
                depth: 5.0,
                reversed: false,
                through_all: false,
                mode: wb_part::ExtrudeMode::Dimension,
                depth2: 0.0,
                taper_deg: 0.0,
                up_to_face: None,
                up_to_offset: 0.0,
            };
            match self
                .document
                .add_feature_in_body(pocket, "Pocket".into(), body)
            {
                Ok(id) => {
                    let _ = pad_id;
                    self.document.mark_feature_dirty(id);
                    self.document.set_feature_visible(top_id, false);
                    self.apply_tree_selection(crate::ui::TreeItemId::Feature(id));
                }
                Err(err) => app_log::error(format!("bench pocket: {err}")),
            }
        }
    }
}

impl PrintCadApp {
    /// The body under the cursor, named with its last feature, and the
    /// point hit on it. Hidden while a button is down or a sketch is being
    /// edited, when the card would only get in the way.
    fn hover_card(&self) -> Option<ui::HoverCard> {
        if self.mouse_buttons_down > 0
            || self.sketch_editing_active()
            || self.viewport_menu.is_some()
        {
            return None;
        }
        let body = core_document::BodyId(self.hovered_body?);
        let point_mm = self.hovered_world_pos?;
        let body_name = self
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())?;
        // The body is named after the last feature that shaped its solid.
        let feature = self
            .document
            .feature_tree()
            .all_nodes()
            .filter(|(_, n)| n.body == Some(body))
            .filter_map(|(id, n)| {
                let info = self.registry.feature_info(n)?;
                info.builds_solid.then_some((n.seq, *id, info.kind_label))
            })
            .max_by_key(|(seq, id, _)| (*seq, *id))
            .map(|(_, _, kind)| kind);
        let title = match feature {
            Some(kind) => format!("{body_name} · {kind}"),
            None => body_name,
        };
        Some(ui::HoverCard { title, point_mm })
    }

    /// "w × h × d" of the selected (else active) body in the display unit.
    fn selection_dimensions(&mut self) -> Option<String> {
        let body = self
            .selected_body
            .map(core_document::BodyId)
            .or(self.active_body_id)?;
        let geometry = self.document.imported_geometry(body)?;
        let bounds = match geometry.bounds_mm {
            Some(bounds) => bounds,
            None => {
                // The mesh scan is linear; keep it per revision.
                match self.dimension_cache {
                    Some((cached_body, revision, bounds))
                        if cached_body == body && revision == geometry.revision =>
                    {
                        bounds
                    }
                    _ => {
                        let bounds = geometry.mesh.bounds()?;
                        self.dimension_cache = Some((body, geometry.revision, bounds));
                        bounds
                    }
                }
            }
        };
        let unit = self.document.display_unit();
        let axes = self.camera.axis_system();
        let lo = axes.world_to_canonical(Vec3::from_array(bounds.0));
        let hi = axes.world_to_canonical(Vec3::from_array(bounds.1));
        let size = (hi - lo).abs();
        Some(format!(
            "{:.1} × {:.1} × {:.1} {}",
            unit.from_mm(size.x),
            unit.from_mm(size.y),
            unit.from_mm(size.z),
            unit.short_label()
        ))
    }
}
