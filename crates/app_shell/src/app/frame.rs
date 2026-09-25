//! Per-frame work: pacing, scene submission assembly, UI run, render, pick.

use std::sync::Arc;
use std::time::{Duration, Instant};

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
/// The build volume as a line body: a box of the bed's width and depth,
/// standing on Z, with the origin at the bed's corner or centre.
fn print_bed_mesh(printing: &settings::PrintingSettings) -> kernel_api::TriMesh {
    let [w, d, h] = printing.bed_mm;
    let (x0, y0) = if printing.origin_center {
        (-w / 2.0, -d / 2.0)
    } else {
        (0.0, 0.0)
    };
    let (x1, y1) = (x0 + w, y0 + d);
    let positions = vec![
        [x0, y0, 0.0],
        [x1, y0, 0.0],
        [x1, y1, 0.0],
        [x0, y1, 0.0],
        [x0, y0, h],
        [x1, y0, h],
        [x1, y1, h],
        [x0, y1, h],
    ];
    let edges = vec![
        0, 1, 1, 2, 2, 3, 3, 0, // bed outline
        4, 5, 5, 6, 6, 7, 7, 4, // top
        0, 4, 1, 5, 2, 6, 3, 7, // uprights
    ];
    kernel_api::TriMesh {
        normals: vec![[0.0, 0.0, 1.0]; positions.len()],
        positions,
        edges,
        ..kernel_api::TriMesh::default()
    }
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

/// The paint of whatever the cursor is over: the hovered edge or face, and
/// the measure tool's marks.
const HOVER_PAINT: [f32; 3] = [1.0, 0.75, 0.2];

/// Frames after geometry arrives at which the bench click hook requests its
/// pick (the camera has settled by then), clicks, activates the bench tool
/// named by `PRINTCAD_BENCH_TOOL`, and reports what the rebuild made of it.
const BENCH_CLICK_PICK_FRAME: u32 = 180;
const BENCH_CLICK_FRAME: u32 = 191;
const BENCH_TOOL_FRAME: u32 = 192;
const BENCH_REPORT_FRAME: u32 = 300;

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
            || self.any_tab_busy()
            || self.file_dialog_rx.is_some()
            || self.export_rx.is_some()
            || !self.nav_device.motion().is_idle()
            || self.script_thread.busy()
            || self.registry.any_busy()
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
            let animating = self.session.camera.is_animating()
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
            self.session.camera.apply_rotate_delta(
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
            self.wait_for_all_document_saves();
            event_loop.exit();
            return;
        }

        // Dev/bench hooks: import a STEP or open a document at startup
        // without a dialog.
        if !self.bench_open_fired {
            self.bench_open_fired = true;
            if let Ok(path) = std::env::var("PRINTCAD_OPEN_FILE") {
                // Several paths, `;`-separated, each import into a tab of
                // its own, so a capture can show the strip; with
                // `PRINTCAD_OPEN_SAME_TAB` they all land in one scene.
                let detail = self.last_step_import_detail.clone();
                let same_tab = std::env::var_os("PRINTCAD_OPEN_SAME_TAB").is_some();
                for path in path.split(';').filter(|p| !p.is_empty()) {
                    if !same_tab {
                        self.ensure_fresh_tab();
                    }
                    self.import_step_at(std::path::Path::new(path), detail.clone());
                }
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
                Ok(n) => self.session.document.bodies().get(n),
                Err(_) => self
                    .session
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.name.contains(&which)),
            }
            .map(|b| (b.id, b.name.clone()))
            && self.session.document.imported_geometry(body.0).is_some()
        {
            self.bench_select_fired = true;
            let row = match self.session.document.imported_object_for_body(body.0) {
                Some(node) => crate::ui::TreeItemId::ImportedObject(node),
                None => crate::ui::TreeItemId::Body(body.0),
            };
            self.apply_tree_selection(row);
            tracing::info!(target: "printcad.frame", "bench selected body {:?} `{}`", body.0, body.1);
        }

        // Dev/bench hook: `PRINTCAD_BENCH_REPAIR=1` asks for the repair of
        // every body the checker calls broken, once, as the tree's menu
        // would, so a run shows the repair land.
        if !self.bench_repair_fired && std::env::var_os("PRINTCAD_BENCH_REPAIR").is_some() {
            let broken: Vec<_> = self
                .session
                .document
                .imported_geometries()
                .filter(|(_, g)| g.health.as_ref().is_some_and(|h| h.is_broken()))
                .map(|(body, _)| *body)
                .collect();
            if !broken.is_empty() {
                self.bench_repair_fired = true;
                self.apply_ui_commands(vec![ui::UiCommand::RepairShapes(broken)], event_loop);
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_CONVERT=1` asks for every mesh
        // body to become a solid, once, as the tree's menu would.
        if !self.bench_convert_fired && std::env::var_os("PRINTCAD_BENCH_CONVERT").is_some() {
            let meshes: Vec<_> = self
                .session
                .document
                .bodies()
                .iter()
                .map(|b| b.id)
                .filter(|b| self.session.document.is_mesh_body(*b))
                .collect();
            if !meshes.is_empty() {
                self.bench_convert_fired = true;
                self.apply_ui_commands(vec![ui::UiCommand::ConvertToSolid(meshes)], event_loop);
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_CLICK=<fx>,<fy>` makes one selection
        // click at that fraction of the viewport once the first body has
        // geometry, and logs what the click saw and what it selected. The
        // camera snaps to a corner view first and settles before the pick is
        // requested, then the readback gets a few frames to land. With
        // `PRINTCAD_BENCH_TOOL=<tool id>` the tool then runs on that
        // selection, as a toolbar click would, and every feature's error is
        // logged once the rebuild has had its turn.
        if let Ok(spec) = std::env::var("PRINTCAD_BENCH_CLICK")
            && self.bench_click_frames <= BENCH_REPORT_FRAME
            && self
                .session
                .document
                .bodies()
                .first()
                .is_some_and(|b| self.session.document.imported_geometry(b.id).is_some())
        {
            self.bench_click_frames += 1;
            self.redraw_needed = true;
            let (fx, fy) = spec
                .split_once(',')
                .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
                .unwrap_or((0.5, 0.5));
            let vp = self.session.camera.viewport_info();
            let cx = fx * vp.2 as f32;
            let cy = fy * vp.3 as f32;
            match self.bench_click_frames {
                1 => self.session.camera.snap_to_view(
                    crate::orientation_cube::CameraSnapView::FrontTopRight,
                    &self.user_settings.camera,
                ),
                BENCH_CLICK_PICK_FRAME..BENCH_CLICK_FRAME => {
                    self.cursor_in_viewport = Some((cx, cy));
                    if let Some(gfx) = self.gfx.as_mut() {
                        gfx.renderer
                            .request_pick((vp.0 + cx) as u32, (vp.1 + cy) as u32);
                    }
                }
                BENCH_CLICK_FRAME => {
                    tracing::info!(
                        target: "printcad.frame",
                        "bench click at ({cx}, {cy}) of {:?}: body {:?} at {:?}, edge {:?}, face {:?}",
                        (vp.2, vp.3),
                        self.session.hovered_body,
                        self.session.hovered_world_pos,
                        self.session.hovered_edge,
                        self.session.hovered_face.as_ref().map(|f| f.face),
                    );
                    self.toggle_body_under_cursor_selection();
                    tracing::info!(
                        target: "printcad.frame",
                        "bench click selected: body {:?}, face {:?}, face highlight {}, edges {}",
                        self.session.selected_body,
                        self.session.last_face_hit,
                        self.session.face_highlight.is_some(),
                        self.session.selected_edges.len(),
                    );
                }
                BENCH_TOOL_FRAME => {
                    if let Ok(tool) = std::env::var("PRINTCAD_BENCH_TOOL") {
                        tracing::info!(target: "printcad.frame", "bench tool {tool}");
                        self.session.active_tool.active_ids.insert(tool);
                    }
                }
                BENCH_REPORT_FRAME => {
                    for (id, node) in self.session.document.feature_tree().all_nodes() {
                        tracing::info!(
                            target: "printcad.frame",
                            "bench feature {:?} `{}` dirty {} error {:?}",
                            id,
                            node.name,
                            node.dirty,
                            node.error,
                        );
                    }
                }
                _ => {}
            }
        }

        let server_status = self.session.server.status();
        let ui_repaint_delay;

        // Pull any STEP imports that the kernel worker finished off the
        // queue before we build this frame's submission, so freshly imported
        // bodies show up immediately and the import log lines stay tied to
        // the frame they actually became visible in. Has to happen before
        // we take a mutable borrow on `self.renderer` below.
        // Every tab takes its turn: a background tab's save completes, its
        // peers' edits land and its solids rebuild while another is on
        // screen.
        self.drain_kernel_responses();
        self.for_each_tab(|app| {
            app.drain_document_saves();
            app.drain_server_messages();
            app.drain_document_opens();
            app.drive_part_recompute();
            app.drive_shape_repairs();
            app.drive_mesh_solids();
        });
        // What formulas moved is followed within the gesture that moved
        // it: a joint an open panel binds re-solves on the next frame.
        self.settle_formulas();
        self.drive_measurement();
        self.drive_scripts(event_loop);
        self.drive_agent_tools();
        self.drive_chats();
        self.refresh_script_library();
        if self.command_ids.is_empty() {
            self.command_ids = self.script_command_ids();
        }

        if self.gfx.is_none() {
            return;
        }

        // Update camera animation and assemble this frame's scene submission
        // before the UI/render block takes its borrows on `gfx`.
        self.call_workbench_on_frame(dt_secs);
        let viewport_data = self.build_scene_submission(dt_secs);
        if self.session.screen == crate::ui::Screen::Start {
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
        let physical = self.panel_physical();
        let host_params = ui::HostCtxParams {
            camera_position: self.session.camera.position(),
            camera_target: self.session.camera.target(),
            viewport: self
                .frame_submission
                .viewport_rect
                .map(|r| (r.x, r.y, r.width, r.height))
                .unwrap_or((0, 0, 1, 1)),
            view_proj: Some(self.session.camera.view_projection()),
            selected_body_id: self.session.active_body_id.map(|id| id.0),
            selected_face: self.session.last_face_hit.as_ref().map(|(_, f)| *f),
            selected_edges: self.selected_edge_refs(),
        };

        let commands;

        {
            let tabs = self.tab_infos();
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
                    camera_orientation: self.session.camera.orientation(),
                    axis_system: self.session.camera.axis_system(),
                };

                let pivot_screen_pos = self
                    .session
                    .camera
                    .rotation_pivot_indicator_screen_px(self.user_settings.camera.orbit_pivot_pick);

                let ui_started = Instant::now();
                let ui_result = ui_layer.run(
                    window,
                    ui::UiFrameInputs {
                        screen: self.session.screen,
                        recent: &self.recent.files,
                        active_tool: self.session.active_tool.clone(),
                        active_workbench: self.session.active_workbench.clone(),
                        settings: &self.user_settings,
                        document: &mut self.session.document,
                        registry: &mut self.registry,
                        host: host_params,
                        orientation_input: Some(&orientation_input),
                        planar_view_lock,
                        fps: (!self.fps_display_idle).then_some(self.current_fps),
                        scene_redraws_per_s: self.scene_redraws_per_s,
                        gpu_name: self.gpu_name.as_deref(),
                        gpus: &self.available_gpus,
                        hovered_point: self.session.hovered_world_pos,
                        pivot_screen_pos,
                        axis_system: self.session.camera.axis_system(),
                        tree_selection: self.session.tree_selection,
                        active_document_object: self.session.active_document_object,
                        editing_feature,
                        viewport_hud,
                        status_items,
                        task,
                        hover_card,
                        dimensions,
                        physical,
                        field_of_view_deg: self.session.camera.field_of_view_deg(),
                        section: self.session.section,
                        scene_bounds: self.session.camera.scene_bounds(),
                        screen_space_overlays: &screen_space_overlays,
                        screen_space_marks: &screen_space_marks,
                        screen_space_labels: &screen_space_labels,
                        pending_imports: self.kernel_worker.in_flight(),
                        pending_document_open: server_status.opens_in_flight
                            + server_status.saves_in_flight
                            + u32::from(self.session.document_open_rx.is_some()),
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
                        document_saving: self.session.document_save_rx.is_some()
                            || server_status.saves_in_flight > 0,
                        save_progress: self.session.save_progress.as_ref().and_then(|p| p.read()),
                        server_label: match (server_status.connected, server_status.peers) {
                            (false, _) => format!("{} (disconnected)", self.session.server.name()),
                            (true, 0) => self.session.server.name().to_string(),
                            (true, 1) => format!("{} · 1 peer", self.session.server.name()),
                            (true, n) => format!("{} · {n} peers", self.session.server.name()),
                        },
                        tabs,
                        measuring: self.session.measure.is_some(),
                        reveal_body: self.session.reveal_body.take(),
                        viewport_menu: self.session.viewport_menu.clone(),
                        nav_device: self.nav_device.device_name(),
                        nav_buttons: self.nav_device.button_count(),
                        step_import_pending: self.session.step_import_pending.as_mut(),
                        export_pending: self.session.export_pending.as_mut(),
                        scripts: &self.script_library,
                        packages: &self.packages,
                        console_attention: std::mem::take(&mut self.console_attention),
                        command_ids: &self.command_ids,
                        script_running: self.script_runs.front().map(|r| r.label.as_str()),
                        recording: self.recording.is_some(),
                        chats: &self.chats,
                        approvals: &self.approvals,
                        assistant_attention: std::mem::take(&mut self.assistant_attention),
                    },
                );
                self.frame_phase_accum.0 += ui_started.elapsed().as_secs_f32() * 1000.0;
                ui_repaint_delay = ui_result.repaint_delay;
                self.frame_submission.egui = Some(ui_result.submission);
                self.session.active_tool = ui_result.active_tool;
                self.session.active_workbench = ui_result.active_workbench;
                self.session.task_open = ui_result.task_open;

                // The window title follows the document and its dirty state.
                let title = if self.session.screen == crate::ui::Screen::Start {
                    "printCAD".to_string()
                } else if self.session.document.metadata().dirty() {
                    format!("• {} - printCAD", self.session.document.name())
                } else {
                    format!("{} - printCAD", self.session.document.name())
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
                self.session.camera.update_viewport(
                    (ui_result.viewport.x, ui_result.viewport.y),
                    (
                        ui_result.viewport.width.max(1),
                        ui_result.viewport.height.max(1),
                    ),
                );

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
                || crate::app::tabs::tabs_busy(&self.session, &self.tabs)
                || self.file_dialog_rx.is_some()
                || self.export_rx.is_some()
                || !self.nav_device.motion().is_idle()
                || self.script_thread.busy()
                || self.registry.any_busy();
            let animating = self.session.camera.is_animating()
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
            self.session.hovered_body = pick_result.body_id;
            self.session.hovered_world_pos = pick_result.world_position;
            self.session.pick_depths = pick_result.depth_window;
        }
        // The edge under the cursor, on the body the pick found; an edge
        // takes the hover from the face it borders.
        let hovered_edge = self.edge_under_cursor();
        if hovered_edge != self.session.hovered_edge {
            self.session.hovered_edge = hovered_edge;
            self.redraw_needed = true;
        }
        if self.refresh_hovered_face() {
            self.redraw_needed = true;
        }

        // Apply this frame's UI actions now that the renderer borrow is over.
        let mut commands = commands;
        commands.extend(self.device_button_commands());
        self.apply_ui_commands(commands, event_loop);
        // A tool clicked in the toolbar acts in the same frame.
        self.dispatch_activated_tools();

        self.publish_presence();

        // Everything this frame edited is in the outbox; hand it to the
        // server. One send per frame keeps drags coalesced (the buffer
        // collapsed them) and the wire quiet when nothing changed. A
        // background tab's outbox carries the edits its peers sent it.
        self.for_each_tab(|app| {
            let ops = app.session.document.take_pending_ops();
            if !ops.is_empty() {
                app.session
                    .server
                    .send(core_document::server::ClientMessage::Ops(ops));
            }
        });

        // Cut an undo boundary at frame end when no drag is in progress so
        // an entire drag interaction coalesces into one step.
        // A task panel's edits stay one gesture until it closes.
        if self.mouse_buttons_down == 0 && !self.session.task_open {
            self.close_gesture();
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
            .workbench(&self.session.active_workbench.0)
            .is_ok_and(|wb| wb.locks_view_to_plane() && wb.editing_feature().is_some())
    }

    fn build_scene_submission(&mut self, dt_secs: f32) -> ViewportData {
        self.session
            .camera
            .set_orbit_lock(self.sketch_editing_active());
        self.session
            .camera
            .flush_pending_wheel(&self.user_settings.camera);
        // Before the clip planes, so they are computed for the pose this
        // frame actually shows.
        let device_motion = self.nav_device.motion();
        self.session.camera.apply_device_motion(
            device_motion.axis_readings(),
            dt_secs,
            &self.user_settings.camera,
            &self.user_settings.sixdof,
        );

        // Every visible feature not under edit draws what its bench says it
        // looks like. The feature under edit is drawn as crisp screen-space
        // overlays by its bench instead (drawing both would double it).
        let editing_feature = self
            .registry
            .workbench(&self.session.active_workbench.0)
            .ok()
            .and_then(|wb| wb.editing_feature());
        let sketch_meshes: Vec<BodySubmission> = self
            .registry
            .passive_geometries(&self.session.document, editing_feature)
            .into_iter()
            .map(|(feature_id, geometry)| {
                // Match the in-edit overlay palette: white geometry,
                // orange hover, green selection. Color is baked directly so
                // the tint is unmistakable even on hairline geometry.
                let is_selected = self.session.active_document_object == Some(feature_id)
                    || self.session.tree_selection
                        == Some(crate::ui::TreeItemId::Feature(feature_id));
                let is_hovered = self.session.hovered_feature == Some(feature_id);
                let color = if is_selected {
                    [0.35, 0.95, 0.45]
                } else if is_hovered {
                    HOVER_PAINT
                } else {
                    [0.85, 0.85, 0.85]
                };
                // The tint participates in the cache revision so hover /
                // selection transitions actually re-upload the color.
                let state_bits = (is_selected as u64) | ((is_hovered as u64) << 1);
                BodySubmission {
                    id: feature_id.0,
                    revision: geometry.revision ^ (state_bits << 62),
                    mesh: Arc::new(geometry.mesh),
                    color,
                    highlight: HighlightState::None,
                    is_wireframe: false,
                    opacity: 1.0,
                    pickable: true,
                    on_top: false,
                }
            })
            .collect();

        // The camera clips to what is drawn now: every visible body, the
        // features drawn beside them and the print bed, never a box kept
        // from the last fit, which a later import or a longer pad outgrows.
        let scene = self.scene_bounds(&sketch_meshes);
        self.session.camera.set_scene_bounds(scene);
        self.session
            .camera
            .apply_auto_clip_planes(&self.user_settings.camera);
        self.session
            .camera
            .update(dt_secs, &self.user_settings.camera);

        // Imported geometry (e.g. STEP files) becomes regular renderable bodies.
        // The body id from the document is reused so picking/selection stays
        // stable, and the document's revision counter is forwarded to the
        // renderer so panning/orbiting never re-uploads the static mesh.
        let draw_style = self.user_settings.rendering.draw_style;
        let wireframe = draw_style == settings::DrawStyle::Wireframe;
        let imported_meshes: Vec<BodySubmission> = self
            .session
            .document
            .imported_geometries()
            .filter(|(body_id, _)| {
                self.session
                    .document
                    .imported_body_effective_visible(**body_id)
            })
            .map(|(body_id, geometry)| {
                // Selection is painted by the overlay below, never by a
                // tint; hover brightens, and a peer's selection tints
                // subordinate to it, so your own interaction always wins.
                let is_hovered = self.session.hovered_body == Some(body_id.0);
                let is_peer_selected = self
                    .session
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
                // A look the user chose wins over the material the body
                // came with; a chosen colour also drops the per-vertex
                // colours, which would tint it.
                let chosen = self
                    .session
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.id == *body_id)
                    .and_then(|b| b.display);
                let use_vertex_albedo = chosen.is_none()
                    && geometry.mesh.colors.len() == geometry.mesh.positions.len()
                    && !geometry.mesh.colors.is_empty();
                let base_color = match chosen {
                    Some(display) => display.color,
                    None if use_vertex_albedo => [1.0, 1.0, 1.0],
                    None => [0.78, 0.78, 0.82],
                };
                let opacity = chosen.map(|d| d.opacity.clamp(0.05, 1.0)).unwrap_or(1.0);
                BodySubmission {
                    id: body_id.0,
                    revision: geometry.revision,
                    mesh: Arc::clone(&geometry.mesh),
                    color: base_color,
                    opacity,
                    highlight,
                    is_wireframe: wireframe,
                    pickable: true,
                    on_top: false,
                }
            })
            .collect();

        let wb_id = self.session.active_workbench.0.clone();

        // Overlay meshes from the active workbench (grid lines, guides, etc.)
        let params = self.overlay_ctx_params();
        let mut overlay_meshes: Vec<BodySubmission> = self
            .with_workbench_ctx(&wb_id, params, |wb, ctx| {
                wb.get_overlay_meshes(ctx, ctx.active_document_object)
            })
            .map(|(meshes, outcome)| {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Lifecycle);
                meshes
            })
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(i, overlay)| {
                // Overlays are regenerated every frame; give slot `i` a
                // stable id from the pool and a content-hash revision so an
                // unchanged overlay is a GPU cache hit instead of a
                // guaranteed upload + GC every frame.
                while self.overlay_id_pool.len() <= i {
                    self.overlay_id_pool.push(Uuid::new_v4());
                }
                BodySubmission {
                    id: self.overlay_id_pool[i],
                    revision: hash_trimesh(&overlay.mesh),
                    mesh: Arc::new(overlay.mesh),
                    color: overlay.color,
                    opacity: overlay.opacity,
                    highlight: HighlightState::None,
                    is_wireframe: overlay.wireframe,
                    // A guide is drawn, never picked.
                    pickable: false,
                    on_top: overlay.on_top,
                }
            })
            .collect();

        // Screen-space overlays + labels from the active workbench
        // (constant-thickness lines and constant-size text).
        let params = self.overlay_ctx_params();
        let mut data = match self.with_workbench_ctx(&wb_id, params, |wb, ctx| ViewportData {
            overlays: wb.get_screen_space_overlays(ctx, ctx.active_document_object),
            marks: wb.get_screen_space_marks(ctx, ctx.active_document_object),
            labels: wb.get_screen_space_labels(ctx, ctx.active_document_object),
            hud: wb.viewport_hud(ctx),
            status: wb.status_items(ctx),
            task: wb.task(ctx),
            editing_feature: wb.editing_feature(),
        }) {
            Some((data, outcome)) => {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Lifecycle);
                data
            }
            None => ViewportData::default(),
        };
        let screen_space_labels = &mut data.labels;

        // The measurement in progress: its points, the line between them
        // and the distance, drawn over the scene.
        if let Some(points) = &self.session.measure {
            let unit = self.session.document.display_unit();
            let color = HOVER_PAINT;
            let px: Vec<(f32, f32)> = points
                .iter()
                .filter_map(|p| self.session.camera.world_to_viewport(Vec3::from_array(*p)))
                .collect();
            for (x, y) in &px {
                data.marks.push(core_document::ScreenSpaceMark::crosshair(
                    [*x, *y],
                    8.0,
                    color,
                ));
            }
            if let ([a, b], [pa, pb]) = (points.as_slice(), px.as_slice()) {
                data.overlays.push(core_document::ScreenSpaceOverlay::new(
                    [pa.0, pa.1],
                    [pb.0, pb.1],
                    color,
                    1.5,
                ));
                let d =
                    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
                screen_space_labels.push(
                    core_document::ScreenSpaceLabel::new(
                        [(pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0 - 12.0],
                        core_document::format_length_mm(d, unit, 2),
                        color,
                        12.0,
                    )
                    .pill(),
                );
            } else if let Some((x, y)) = self.cursor_in_viewport {
                let prompt = if points.is_empty() {
                    "Measure: pick the first point"
                } else {
                    "Measure: pick the second point"
                };
                screen_space_labels.push(
                    core_document::ScreenSpaceLabel::new(
                        [x + 14.0, y + 14.0],
                        prompt.to_string(),
                        color,
                        11.0,
                    )
                    .pill(),
                );
            }
        }

        // Peers' cursors: a named marker where each other editor points.
        // Same projection the overlays use; a cursor behind the camera or
        // outside the model simply has no marker this frame.
        for state in self.session.peer_presence.values() {
            let Some(world) = state.cursor_world else {
                continue;
            };
            let Some((x, y)) = self
                .session
                .camera
                .world_to_viewport(Vec3::new(world[0], world[1], world[2]))
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
        if let Some(face) = &self.session.face_highlight {
            all_meshes.push(BodySubmission {
                id: self.face_highlight_id,
                revision: face.revision,
                mesh: Arc::clone(&face.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
            });
        } else if let Some(geometry) = self
            .session
            .selected_body
            // Picked edges are the selection then, drawn as lines below.
            .filter(|_| self.session.selected_edges.is_empty())
            .and_then(|id| {
                self.session
                    .document
                    .imported_geometry(core_document::BodyId(id))
            })
        {
            // One slot serves every body; the body's id in the revision
            // keeps two bodies at the same revision from sharing buffers.
            let (hi, lo) = self.session.selected_body.unwrap_or_default().as_u64_pair();
            all_meshes.push(BodySubmission {
                id: self.body_highlight_id,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: paint,
                opacity,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
            });
        }

        // The hovered face, translucent in the hover paint, unless it is the
        // selected face already.
        if let Some(hover) = &self.session.hovered_face
            && self.session.hovered_edge.is_none()
            && !self
                .session
                .face_highlight
                .as_ref()
                .is_some_and(|f| f.body == hover.body && f.face == Some(hover.face))
        {
            let (hi, lo) = hover.body.as_u64_pair();
            all_meshes.push(BodySubmission {
                id: self.face_hover_id,
                revision: hover.revision ^ hi ^ lo ^ (u64::from(hover.face) << 32),
                mesh: Arc::clone(&hover.mesh),
                color: HOVER_PAINT,
                opacity: opacity * 0.5,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
            });
        }

        // The hovered edge and the picked edges, drawn as line bodies over
        // the outline in the hover and selection paints.
        if let Some(hit) = &self.session.hovered_edge
            && !self
                .session
                .selected_edges
                .iter()
                .any(|s| s.body == hit.body && s.edge == hit.edge)
            && let Some(submission) = crate::app::edges::highlight_submission(
                self,
                self.edge_hover_id,
                hit.body,
                &[hit.edge],
                HOVER_PAINT,
            )
        {
            all_meshes.push(submission);
        }
        let mut by_body: std::collections::BTreeMap<uuid::Uuid, Vec<u32>> = Default::default();
        for hit in &self.session.selected_edges {
            by_body.entry(hit.body).or_default().push(hit.edge);
        }
        for (i, (body, edges)) in by_body.iter().enumerate() {
            // One slot per body: its id folded into the submission id.
            let id = uuid::Uuid::from_u64_pair(
                self.edge_select_id.as_u64_pair().0 ^ body.as_u64_pair().0,
                self.edge_select_id.as_u64_pair().1.wrapping_add(i as u64),
            );
            if let Some(submission) =
                crate::app::edges::highlight_submission(self, id, *body, edges, paint)
            {
                all_meshes.push(submission);
            }
        }

        // The printer's build volume, as twelve lines around the model.
        if self.user_settings.printing.show_bed {
            let printing = &self.user_settings.printing;
            let revision = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for v in printing.bed_mm {
                    v.to_bits().hash(&mut h);
                }
                printing.origin_center.hash(&mut h);
                h.finish()
            };
            if self
                .print_bed
                .as_ref()
                .is_none_or(|(rev, _)| *rev != revision)
            {
                self.print_bed = Some((revision, Arc::new(print_bed_mesh(printing))));
            }
            if let Some((_, mesh)) = &self.print_bed {
                all_meshes.push(BodySubmission {
                    id: self.print_bed_id,
                    revision,
                    mesh: Arc::clone(mesh),
                    color: [0.45, 0.52, 0.6],
                    opacity: 1.0,
                    highlight: HighlightState::None,
                    is_wireframe: false,
                    pickable: false,
                    on_top: false,
                });
            }
        }

        self.frame_submission.bodies = all_meshes;
        self.frame_submission.draw_edges = draw_style == settings::DrawStyle::ShadedEdges;
        self.frame_submission.clip_plane = self.session.section.map(|plane| plane.equation());
        self.frame_submission.view_proj = self.session.camera.view_projection();
        self.frame_submission.camera_pos = self.session.camera.position();
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
    /// editing, so the sketcher can be exercised without clicking; `pad`
    /// pads it and selects the pad, `pocket` pockets the pad's top too.
    fn bench_open_sketch(&mut self) {
        self.create_new_body();
        let scene = bench_fixtures::Scene::named(
            &std::env::var("PRINTCAD_BENCH_SKETCH").unwrap_or_default(),
        );
        match bench_fixtures::open_sketch_scene(
            &mut self.session.document,
            self.session.active_body_id,
            scene,
        ) {
            Ok(handles) => {
                if let Some(feature) = handles.activate {
                    self.apply_tree_activation(crate::ui::TreeItemId::Feature(feature));
                }
                if let Some(feature) = handles.select {
                    self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature));
                }
            }
            Err(err) => app_log::error(format!("bench scene: {err}")),
        }
    }
}

impl PrintCadApp {
    /// The box around everything the scene pass draws: visible bodies,
    /// the features drawn beside them, and the print bed when it shows.
    /// `None` for an empty scene.
    fn scene_bounds(&self, features: &[BodySubmission]) -> Option<(Vec3, Vec3)> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        let mut add = |(a, b): ([f32; 3], [f32; 3])| {
            lo = lo.min(Vec3::from_array(a));
            hi = hi.max(Vec3::from_array(b));
        };
        let document = &self.session.document;
        for (body, geometry) in document.imported_geometries() {
            if document.imported_body_effective_visible(*body)
                && let Some(bounds) = geometry.bounds_mm.or_else(|| geometry.mesh.bounds())
            {
                add(bounds);
            }
        }
        for feature in features {
            if let Some(bounds) = feature.mesh.bounds() {
                add(bounds);
            }
        }
        let printing = &self.user_settings.printing;
        if printing.show_bed {
            let [w, d, h] = printing.bed_mm;
            let (x0, y0) = if printing.origin_center {
                (-w / 2.0, -d / 2.0)
            } else {
                (0.0, 0.0)
            };
            add(([x0, y0, 0.0], [x0 + w, y0 + d, h]));
        }
        (lo.x <= hi.x).then_some((lo, hi))
    }

    /// Resolve the face under the cursor from the pick, keeping the copy
    /// already made when the picked point has not moved. Returns whether the
    /// hover changed.
    fn refresh_hovered_face(&mut self) -> bool {
        let probe = self
            .session
            .hovered_body
            .zip(self.session.hovered_world_pos)
            .filter(|_| !self.sketch_editing_active() && self.mouse_buttons_down == 0);
        let Some((body, point)) = probe else {
            return self.session.hovered_face.take().is_some();
        };
        if self
            .session
            .hovered_face
            .as_ref()
            .is_some_and(|h| h.body == body && h.probe == point)
        {
            return false;
        }
        let resolved = self
            .session
            .document
            .imported_geometry(core_document::BodyId(body))
            .and_then(|geometry| {
                let face = crate::app::input::face_id_at(&geometry.mesh, point)?;
                if let Some(h) = &self.session.hovered_face
                    && h.body == body
                    && h.face == face
                    && h.revision == geometry.revision
                {
                    // The same face at a new point: keep the copy.
                    return Some(crate::app::input::FaceHover {
                        body,
                        face,
                        revision: h.revision,
                        mesh: Arc::clone(&h.mesh),
                        probe: point,
                    });
                }
                let mesh = crate::app::input::face_submesh_by_id(&geometry.mesh, face)?;
                Some(crate::app::input::FaceHover {
                    body,
                    face,
                    revision: geometry.revision,
                    mesh: Arc::new(mesh),
                    probe: point,
                })
            });
        let changed = match (&self.session.hovered_face, &resolved) {
            (Some(a), Some(b)) => a.body != b.body || a.face != b.face,
            (None, None) => false,
            _ => true,
        };
        self.session.hovered_face = resolved;
        changed
    }

    /// The body under the cursor, named with its last feature, and the
    /// point hit on it. Hidden while a button is down or a sketch is being
    /// edited, when the card would only get in the way.
    fn hover_card(&self) -> Option<ui::HoverCard> {
        if self.mouse_buttons_down > 0
            || self.sketch_editing_active()
            || self.session.viewport_menu.is_some()
        {
            return None;
        }
        // An edge hovered from just outside a silhouette has no surface
        // under the cursor; the card then speaks for the edge's body.
        let (body, point_mm) = match self.session.hovered_edge {
            Some(edge) => (core_document::BodyId(edge.body), edge.point),
            None => (
                core_document::BodyId(self.session.hovered_body?),
                self.session.hovered_world_pos?,
            ),
        };
        let body_name = self
            .session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())?;
        // The body is named after the last feature that shaped its solid.
        let feature = self
            .session
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
        let title = match (&self.session.hovered_edge, feature) {
            (Some(edge), _) => format!(
                "{body_name} · edge {}",
                core_document::format_length_mm(
                    edge.length_mm,
                    self.session.document.display_unit(),
                    2
                )
            ),
            (None, Some(kind)) => format!("{body_name} · {kind}"),
            (None, None) => body_name,
        };
        Some(ui::HoverCard { title, point_mm })
    }

    /// "w × h × d" of the selected (else active) body in the display unit.
    fn selection_dimensions(&mut self) -> Option<String> {
        let body = self
            .session
            .selected_body
            .map(core_document::BodyId)
            .or(self.session.active_body_id)?;
        let geometry = self.session.document.imported_geometry(body)?;
        let bounds = match geometry.bounds_mm {
            Some(bounds) => bounds,
            None => {
                // The mesh scan is linear; keep it per revision.
                match self.session.dimension_cache {
                    Some((cached_body, revision, bounds))
                        if cached_body == body && revision == geometry.revision =>
                    {
                        bounds
                    }
                    _ => {
                        let bounds = geometry.mesh.bounds()?;
                        self.session.dimension_cache = Some((body, geometry.revision, bounds));
                        bounds
                    }
                }
            }
        };
        let unit = self.session.document.display_unit();
        let axes = self.session.camera.axis_system();
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
