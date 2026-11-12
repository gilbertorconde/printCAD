//! Per-frame work: pacing, scene submission assembly, UI run, render, pick.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

/// The render slot a scan's marked surface draws in.
const MARKING_SLOT: u128 = 0x6d61_726b_696e_6700_0000_0000_0000_0001;

use glam::Vec3;
use render_wgpu::{
    BodySubmission, GpuLight, HighlightState, LightingData, RenderBackend,
    ViewportRect as RenderViewportRect,
};
use settings::{ProjectionMode, SixDofButtonAction, UserSettings};
use uuid::Uuid;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use super::scene_guides::{self, GuideView};
use crate::log_panel as app_log;
use crate::orientation_cube::{CameraSnapView, OrientationCubeInput};
use crate::{Document, PrintCadApp, ui};

mod scene;

/// A bench overlay's revision as the renderer keys it: the bench's own,
/// told apart from every other bench's.
fn overlay_revision(bench: &impl std::hash::Hash, revision: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (bench, revision).hash(&mut hasher);
    hasher.finish()
}

/// The build volume as a line body: a box of the bed's width and depth,
/// standing on Z, with the origin at the bed's corner or centre.
/// The mesh a linked copy shares with its source, in their own frame:
/// `None` for any other body, a mirrored copy, or a copy whose mesh is
/// its own.
fn shared_mesh(
    document: &Document,
    body: core_document::BodyId,
) -> Option<Arc<kernel_api::TriMesh>> {
    let entry = document.body(body)?;
    let source = entry.copy_of.filter(|_| entry.mirror.is_none())?;
    let (own, _) = document.local_geometry(body)?;
    let (theirs, _) = document.local_geometry(source)?;
    Arc::ptr_eq(&own, &theirs).then_some(own)
}

pub(crate) fn print_bed_mesh(printing: &settings::PrintingSettings) -> kernel_api::TriMesh {
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

/// How long after the last input frames keep coming, so egui's reactions
/// and the pick readbacks land.
const INPUT_TAIL: Duration = Duration::from_millis(150);

/// Mixed into a feature's id for the id of the region it shades, so the
/// two never share a renderer cache slot.
const REGION_ID_SALT: u128 = 0x5245_4749_4f4e_5f53_4841_4445_0000_0001;
/// Folded into a feature's id for the lines of its parts picked on their
/// own.
const PART_LINES_ID_SALT: u128 = 0x5041_5254_5f4c_494e_4553_0000_0000_0001;
/// Folded into a feature's id for the lines of its curve under the cursor.
const HOVERED_LINES_ID_SALT: u128 = 0x484f_5645_525f_4c49_4e45_5300_0000_0001;
/// How much stronger than the feature's strongest region a region picked
/// or under the cursor is shaded.
const LIT_REGION_GAIN: f32 = 3.5;

impl PrintCadApp {
    /// The body the tree row under the pointer stands for: a body's row, an
    /// imported part's, or a feature's that builds its body's solid. A
    /// hidden body lights nothing.
    fn tree_hovered_body(&self) -> Option<core_document::BodyId> {
        use crate::ui::TreeItemId;
        let document = &self.session.document;
        let body = match self.session.tree_hovered? {
            TreeItemId::Body(id) => Some(id),
            TreeItemId::ImportedObject(node) => document.body_of_imported_object(node),
            TreeItemId::Feature(id) => document.get_feature_meta(id).and_then(|node| {
                self.registry
                    .feature_info(node)
                    .filter(|info| info.builds_solid)
                    .and(node.body)
            }),
            _ => None,
        }?;
        document
            .imported_body_effective_visible(body)
            .then_some(body)
    }

    /// A picture of the view that arrived: saved where the user picks.
    fn drive_picture(&mut self) {
        let Some(picture) = self.picture.take() else {
            return;
        };
        if let Some(path) = &crate::app::bench_hooks::hooks().picture {
            let written =
                crate::app::animation::png_of(picture.width, picture.height, &picture.rgba)
                    .and_then(|png| std::fs::write(path, png).map_err(|e| e.to_string()));
            tracing::info!(target: "printcad.frame", "bench picture {:?}: {:?}", path, written);
            return;
        }
        match crate::app::animation::png_of(picture.width, picture.height, &picture.rgba) {
            Ok(contents) => {
                self.start_file_dialog(crate::app::doc_io::FileDialogKind::SaveFile(Box::new(
                    crate::app::doc_io::FileToSave {
                        name: format!("{}.png", self.session.document.name()),
                        kind: "PNG picture".into(),
                        extension: "png".into(),
                        contents,
                    },
                )));
            }
            Err(why) => app_log::error(format!("Could not make the picture: {why}")),
        }
    }

    /// Every background source that must keep frames coming until it
    /// settles, `capture_pending` the renderer's picture readback. The wake
    /// gate and the end-of-frame scheduler share this: a source listed in
    /// only one of them either burns CPU or sleeps through its own
    /// completion.
    fn async_work_pending(&self, capture_pending: bool) -> bool {
        capture_pending
            || self.texture_previews_pending()
            || self.session.mesh_histories.pending()
            || self.picture.is_some()
            || self.kernel_worker.in_flight() > 0
            || self.any_tab_busy()
            || self.file_dialog_rx.is_some()
            || self.export_rx.is_some()
            || !self.nav_device.motion().is_idle()
            || self.script_thread.busy()
            || self.registry.any_busy()
            || self.package_work.busy()
            || self.session.gestures.radial_press.is_some()
    }

    /// Whether the view moves on its own: a camera tween in either pane,
    /// or a bench hook that keeps frames coming.
    fn animating(&self) -> bool {
        self.session.camera.is_animating()
            || self
                .session
                .side_camera
                .as_ref()
                .is_some_and(|c| c.is_animating())
            || crate::app::bench_hooks::hooks().animating()
    }

    /// Ask the window for another frame.
    fn request_redraw(&self) {
        if let Some(gfx) = &self.gfx {
            gfx.window.request_redraw();
        }
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

    /// Body of `about_to_wait`: pace the frame, drain worker channels,
    /// assemble the scene, run the UI, render, read back the pick, and
    /// apply this frame's UI commands.
    pub(crate) fn frame(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        // When the loop must wake for the next autosave or to keep the
        // session, idle or not.
        let timed_at = self
            .next_autosave()
            .into_iter()
            .chain(self.next_session_keeping())
            .min();
        self.radial_tick();
        self.count_tool_uses();
        crate::app::crash::sync(
            self.session.tab,
            &mut self.session.document,
            self.session.current_file.as_deref(),
        );
        // Optional FPS cap from settings (0 = uncapped). Timing and FPS
        // advance only on a frame that is rendered.
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
        // frame callbacks acknowledging the app's own presents. Render only
        // when someone asked (scheduler, input, OS expose) or state demands
        // it: otherwise presenting would wake the loop again and it never
        // rests.
        {
            let input_active = self
                .last_input_time
                .is_some_and(|t| t.elapsed() < INPUT_TAIL);
            let animating = self.animating();
            let timed_due = timed_at.is_some_and(|at| at <= now);
            if !(self.redraw_needed
                || input_active
                || self.async_work_pending(
                    self.gfx
                        .as_ref()
                        .is_some_and(|gfx| gfx.renderer.capture_pending()),
                )
                || animating
                || timed_due
                || self.pending_ui_repaint.is_zero())
            {
                // Asleep, the loop still wakes for the next timed work.
                event_loop.set_control_flow(match timed_at {
                    Some(at) => ControlFlow::WaitUntil(at),
                    None => ControlFlow::Wait,
                });
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
                // The display updates every frame from a smoothed frame time:
                // a real number one frame after waking, without the jitter
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

        if !self.drive_bench_hooks(event_loop) {
            return;
        }

        let server_status = self.session.server.status();
        let ui_repaint_delay;
        let capture_pending;

        // Pull any STEP imports that the kernel worker finished off the
        // queue before this frame's submission is built, so freshly imported
        // bodies show up immediately and the import log lines stay tied to
        // the frame they actually became visible in. Has to happen before
        // the mutable borrow on `self.renderer` below.
        // Every tab takes its turn: a background tab's save completes, its
        // peers' edits land and its solids rebuild while another is on
        // screen.
        // The workspace drawn last frame laid the viewport out: the opened
        // document is framed in it once its bodies have geometry.
        if self.session.fit_on_layout
            && self.session.screen == crate::ui::Screen::Workspace
            && crate::app::view_fit::document_imported_aabb(&self.session.document).is_some()
        {
            self.session.fit_on_layout = false;
            self.frame_scene();
            self.redraw_needed = true;
        }
        self.drain_kernel_responses();
        self.for_each_tab(|app| {
            app.drain_document_saves();
            app.drain_server_messages();
            app.drain_document_opens();
            app.drive_part_recompute();
            app.drive_shape_repairs();
            app.drive_shape_refinements();
            app.drive_shape_replacements();
            app.drive_mesh_solids();
            app.drive_mirrored_copies();
            app.drive_links();
        });
        // What formulas moved is followed within the gesture that moved
        // it: a joint an open panel binds re-solves on the next frame.
        self.settle_formulas();
        self.drive_measurement();
        self.drive_scripts(event_loop);
        self.drive_agent_tools();
        self.drive_chats();
        self.drive_picture();
        self.drive_autosave();
        self.drive_session_keeping();
        self.drive_bench_settings_keeping();
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
            self.frame_submission.grids.clear();
        }
        let ViewportData {
            overlays: screen_space_overlays,
            polygons: screen_space_polygons,
            images: screen_space_images,
            marks: screen_space_marks,
            labels: screen_space_labels,
            hud: viewport_hud,
            scalar_maps,
            status: status_items,
            task,
            editing_feature,
            clip_plane: _,
            view_grid: _,
            faded: _,
        } = viewport_data;
        let planar_view_lock = self.sketch_editing_active();
        let hover_card = self.hover_card();
        let picks = self.picks_summary();
        let picked_faces = self.picked_face_ids();
        let dimensions = self.selection_dimensions();
        let quick_measure = self.drive_quick_measure();
        self.follow_section_formula();
        let selection_mass = self.selection_mass();
        let physical = self.panel_physical();
        let inertia_references = self.inertia_references();
        let host_params = ui::HostCtxParams {
            pixels_per_point: self.pixels_per_point(),
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
            selected_faces: self.selected_face_refs(),
            selected_edges: self.selected_edge_refs(),
            selected_parts: self.session.selected_parts(),
            selected_features: self.session.selected_features(),
            bought_kinds: self.registry.bought_kinds(),
            undo_entry: self.session.journal.undo_entry_id(),
        };

        let commands;
        let card_shown;
        // The cursor is on the scene itself, not on a menu or card over it.
        let over_scene;

        {
            let tabs = self.tab_infos();
            let unsaved_question = self.unsaved_question();
            let save_over = self.save_over_question();
            let read_only_file = self.current_file_read_only();
            let marking = self.marking_counts();
            let note_card = self.note_card_view();
            let mut mentions: Vec<(Uuid, Vec<crate::app::mentions::MentionOption>)> = Vec::new();
            for chat in &self.chats {
                if !mentions.iter().any(|(tab, _)| *tab == chat.tab) {
                    mentions.push((chat.tab, self.mention_options(chat.tab)));
                }
            }
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

                let tree_also = self.session.tree_also().to_vec();
                // The feature that made the face picked in the view.
                let face_maker = self
                    .session
                    .last_face_hit
                    .filter(|_| self.session.face_highlight.is_some())
                    .and_then(|(body, face)| {
                        self.session
                            .document
                            .face_maker(core_document::BodyId(body), face.name)
                    });
                let measure_bar = self.session.measure.as_ref().map(|picks| {
                    crate::ui::view_toolbar::MeasureBar {
                        unit: self.session.document.measure_unit(),
                        document_unit: self.session.document.display_unit(),
                        picks: picks.len(),
                    }
                });
                let ui_started = Instant::now();
                let ui_result = ui_layer.run(
                    window,
                    ui::UiFrameInputs {
                        screen: self.session.screen,
                        split_view: self.session.split_view
                            && self
                                .session
                                .document
                                .bodies()
                                .iter()
                                .any(|b| self.session.document.body_scan(b.id).is_some()),
                        split_linked: self.session.side_camera.is_none(),
                        recent: &self.recent.files,
                        recoverable: &self.recoverable,
                        unsaved_question,
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
                        graphics_api: self.graphics_api,
                        gpus: &self.available_gpus,
                        hovered_point: self.session.hovered_world_pos,
                        pivot_screen_pos,
                        axis_system: self.session.camera.axis_system(),
                        tree_selection: self.session.tree_selection,
                        tree_also: &tree_also,
                        active_document_object: self.session.active_document_object,
                        editing_feature,
                        viewport_hud,
                        scalar_legend: scalar_maps.first(),
                        status_items,
                        task,
                        hover_card,
                        picks,
                        dimensions,
                        quick_measure,
                        selection_mass,
                        physical,
                        inertia_references,
                        field_of_view_deg: self.session.camera.field_of_view_deg(),
                        projection: self.session.camera.projection(),
                        section: self.session.section.clone(),
                        section_prompt: self
                            .session
                            .section_pick
                            .as_ref()
                            .map(|pick| pick.prompt()),
                        pick_filter: self.session.pick_filter,
                        scene_bounds: self.session.camera.scene_bounds(),
                        screen_space_overlays: &screen_space_overlays,
                        screen_space_polygons: &screen_space_polygons,
                        screen_space_images: &screen_space_images,
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
                        server: crate::ui::status_bar::ServerBadge {
                            name: self.session.server.name().to_string(),
                            standalone: self.session.server.standalone(),
                            connected: server_status.connected,
                            peers: server_status.peers,
                        },
                        tabs,
                        measuring: self.session.measure.is_some(),
                        radial: self.session.gestures.radial.as_ref().map(|radial| {
                            let scale = self.session.camera.pixels_per_point();
                            let cursor = self
                                .cursor_in_viewport
                                .map_or(radial.centre, |(x, y)| glam::Vec2::new(x, y));
                            ui::radial::RadialView {
                                centre: radial.centre.to_array(),
                                slices: radial
                                    .slices
                                    .iter()
                                    .map(|s| (s.icon, s.label.clone()))
                                    .collect(),
                                pointed: radial.pointed(cursor, scale),
                            }
                        }),
                        measure_bar,
                        note_tool: self.session.note_tool,
                        note_card,
                        reveal_body: self.session.reveal_body.take(),
                        face_maker,
                        reveal_face_maker: std::mem::take(&mut self.session.reveal_face_maker),
                        viewport_menu: self.session.viewport_menu.clone(),
                        picked_faces,
                        picked_edges: self
                            .session
                            .selected_edges
                            .iter()
                            .map(|e| (core_document::BodyId(e.body), e.point))
                            .collect(),
                        nav_device: self.nav_device.device_name(),
                        nav_buttons: self.nav_device.button_count(),
                        step_import_pending: self.session.step_import_pending.as_mut(),
                        export_pending: self.session.export_pending.as_mut(),
                        scripts: &self.script_library,
                        packages: &self.packages,
                        release: &self.release_check,
                        store: &self.store,
                        console_attention: std::mem::take(&mut self.console_attention),
                        command_ids: &self.command_ids,
                        script_running: self.script_runs.front().map(|r| r.label.as_str()),
                        recording: self.recording.is_some(),
                        chats: &self.chats,
                        approvals: &self.approvals,
                        mentions,
                        assistant_attention: std::mem::take(&mut self.assistant_attention),
                        paste_pressed: std::mem::take(&mut self.paste_pressed),
                        print_layout: self.session.print_layout.as_ref().map(|s| &s.layout),
                        marking,
                        file: crate::ui::FileMarks {
                            read_only: read_only_file,
                            read_only_links: &self.session.links.read_only,
                            newer_notice: self
                                .session
                                .file_safety
                                .newer_notice
                                .then(|| self.session.file_safety.newer_text()),
                            save_over,
                            mesh_units: self.session.mesh_units_pending.as_ref().map(|path| {
                                (
                                    path.file_name().map_or_else(
                                        || path.display().to_string(),
                                        |n| n.to_string_lossy().into_owned(),
                                    ),
                                    self.user_settings.files.mesh_units,
                                )
                            }),
                        },
                    },
                );
                self.frame_phase_accum.0 += ui_started.elapsed().as_secs_f32() * 1000.0;
                ui::widget_map::publish();
                ui_repaint_delay = ui_result.repaint_delay;
                self.frame_submission.egui = Some(ui_result.submission);
                // A tool that started this frame is what Repeat starts again.
                if let Some(started) = ui_result.active_tool.active_ids.iter().find(|id| {
                    !self.session.active_tool.active_ids.contains(*id)
                        && !core_document::base_tool_id(id).ends_with(".select")
                }) {
                    self.session.last_tool =
                        Some((ui_result.active_workbench.0.clone(), started.clone()));
                }
                self.session.active_tool = ui_result.active_tool;
                self.session.active_workbench = ui_result.active_workbench;
                self.session.task_open = ui_result.task_open;
                card_shown = ui_result.card;
                self.session.print_layout_shown = ui_result.print_layout_open;
                if self.session.mark_arm != ui_result.mark_arm {
                    self.session.mark_arm = ui_result.mark_arm;
                    self.session.gestures.take_mark();
                }
                self.session.tree_hovered = ui_result.tree_hovered;

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
                // The pane beside the viewport shows every body but the
                // scans, and the selection and hover paints drawn for them,
                // with the viewport's camera or its own.
                self.session.side_rect = ui_result
                    .side_viewport
                    .map(|r| (r.x, r.y, r.width.max(1), r.height.max(1)));
                if let (Some(camera), Some((x, y, w, h))) =
                    (self.session.side_camera.as_mut(), self.session.side_rect)
                {
                    camera.update_viewport((x, y), (w, h));
                }
                let document = &self.session.document;
                let side_camera = self.session.side_camera.as_ref();
                let overlays = &self.session.overlay_bodies;
                self.frame_submission.side_pane = ui_result.side_viewport.map(|r| {
                    let mut bodies: Vec<Uuid> = document
                        .bodies()
                        .iter()
                        .filter(|b| document.body_scan(b.id).is_none())
                        .map(|b| b.id.0)
                        .collect();
                    bodies.extend(
                        overlays
                            .iter()
                            .filter(|(_, body)| bodies.contains(body))
                            .map(|(id, _)| *id)
                            .collect::<Vec<_>>(),
                    );
                    render_wgpu::SidePane {
                        rect: RenderViewportRect {
                            x: r.x,
                            y: r.y,
                            width: r.width,
                            height: r.height,
                        },
                        bodies,
                        camera: side_camera.map(|c| render_wgpu::PaneCamera {
                            view_proj: c.view_projection(),
                            camera_pos: c.position(),
                        }),
                    }
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
            for picture in renderer.take_captures() {
                match picture.kind {
                    render_wgpu::CaptureKind::Scene => self.picture = Some(picture),
                    render_wgpu::CaptureKind::Window => {
                        let path = crate::app::bench_hooks::hooks()
                            .window
                            .as_deref()
                            .unwrap_or(std::path::Path::new(""));
                        let written = crate::app::animation::png_of(
                            picture.width,
                            picture.height,
                            &picture.rgba,
                        )
                        .and_then(|png| std::fs::write(path, png).map_err(|e| e.to_string()));
                        tracing::info!(target: "printcad.frame", "bench window {:?}: {:?}", path, written);
                    }
                    render_wgpu::CaptureKind::View => {
                        let answer = crate::app::mcp::screen_answer(&picture);
                        for reply in self.screen_waiting.drain(..) {
                            let _ = reply.send(answer.clone());
                        }
                    }
                }
            }

            capture_pending = renderer.capture_pending();

            // Retrieve pick result from GPU picking (processed during render).
            // The pick answers for where the cursor last was on the scene; a
            // menu or card opened under a still cursor, or the cursor gone
            // off the view, leaves nothing hovered, rather than the last
            // answer coming back each frame.
            over_scene = (self.cursor_in_viewport.is_some() || self.cursor_in_side.is_some())
                && !ui_layer.pointer_over_floating_ui();
            let pick_result = renderer.latest_pick_result();
            self.session.hovered_body = pick_result.body_id.filter(|_| over_scene);
            self.session.hovered_world_pos = pick_result.world_position.filter(|_| over_scene);
            self.session.hovered_triangle = pick_result
                .triangle
                .zip(self.session.hovered_body)
                .and_then(|(triangle, body)| {
                    crate::app::input::document_triangle(
                        triangle,
                        self.session.drawn_meshes.get(&body).copied(),
                    )
                });
            self.session.pick_depths = pick_result.depth_window;
        }
        // Render on demand: another frame is scheduled only while
        // something is moving, pending, or animating. A short tail after
        // input lets egui reactions and pick readbacks land; a frame
        // drawn without edges gets one more to restore them; egui's own
        // timed repaints (caret blink) become a timed wake-up. Otherwise
        // the loop sleeps until the next OS event.
        let input_active = self
            .last_input_time
            .is_some_and(|t| t.elapsed() < INPUT_TAIL);
        let work_pending = self.async_work_pending(capture_pending);
        let animating = self.animating();
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
            self.request_redraw();
            event_loop.set_control_flow(ControlFlow::Wait);
        } else if !self.fps_display_idle {
            // Going idle: paint one closing frame whose FPS display says
            // so, then sleep for real.
            self.fps_display_idle = true;
            self.redraw_needed = true;
            self.request_redraw();
            event_loop.set_control_flow(ControlFlow::Wait);
        } else if ui_repaint_delay < Duration::from_secs(10) {
            // The timed wake (caret blink etc.) should render once.
            self.redraw_needed = true;
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + ui_repaint_delay));
        } else if fps_cap <= 0.0 {
            event_loop.set_control_flow(match timed_at {
                Some(at) => ControlFlow::WaitUntil(at),
                None => ControlFlow::Wait,
            });
        }

        let hovered_plane = self.base_plane_under_cursor().filter(|_| over_scene);
        if hovered_plane != self.session.hovered_base_plane {
            self.session.hovered_base_plane = hovered_plane;
            self.redraw_needed = true;
        }
        // The edge under the cursor, on the body the pick found; an edge
        // takes the hover from the face it borders.
        let hovered_edge = self
            .edge_under_cursor()
            .filter(|_| over_scene && self.session.pick_filter.edges());
        if hovered_edge != self.session.hovered_edge {
            self.session.hovered_edge = hovered_edge;
            self.redraw_needed = true;
        }
        if self.refresh_hovered_face() {
            self.redraw_needed = true;
        }
        self.session.hovered_face_ref = self
            .session
            .hovered_body
            .filter(|_| over_scene)
            .and_then(|body| self.face_ref_under_cursor(body));
        self.session.hovered_vertex = self.vertex_under_cursor();

        // Apply this frame's UI actions now that the renderer borrow is over.
        self.follow_card(card_shown);
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
        // A task's or a floating card's edits stay one gesture until it
        // closes.
        let task_under_way = self.follow_task_step();
        if self.mouse_buttons_down == 0 && !task_under_way && self.session.card.is_none() {
            self.close_gesture();
        }
    }

    /// True while the active workbench has an edit session open that keeps
    /// the view square to its plane. Gates the camera's out-of-plane
    /// rotation and the hover feedback that would compete with the bench's
    /// own.
    pub(crate) fn sketch_editing_active(&self) -> bool {
        self.registry
            .workbench(&self.session.active_workbench.0)
            .is_ok_and(|wb| wb.locks_view_to_plane() && wb.editing_feature().is_some())
    }

    /// Whether the active bench asks for the origin's planes to be picked
    /// from.
    pub(crate) fn bench_shows_origin_planes(&self) -> bool {
        self.registry
            .workbench(&self.session.active_workbench.0)
            .is_ok_and(|wb| wb.shows_origin_planes())
    }

    /// The origin plane under the cursor while the active bench shows them,
    /// unless a body the pick found stands in front of it.
    fn base_plane_under_cursor(&self) -> Option<core_document::BasePlane> {
        if !self.bench_shows_origin_planes() {
            return None;
        }
        let (origin, dir) = self.cursor_ray()?;
        let half = scene_guides::origin_plane_half(&GuideView::of(&self.session.camera));
        let (plane, t) = scene_guides::origin_plane_hit(origin, dir, half)?;
        let in_front = self
            .session
            .hovered_world_pos
            .is_some_and(|p| (glam::Vec3::from_array(p) - origin).dot(dir) < t);
        (!in_front).then_some(plane)
    }

    /// Update the camera and assemble this frame's [`FrameSubmission`]
    /// (sketch tessellations, imported bodies, workbench overlay meshes).
    /// Returns what the active workbench contributes beyond the 3D pass:
    /// its screen-space overlays, drawn via egui, and the rest of
    /// [`ViewportData`].
    fn build_scene_submission(&mut self, dt_secs: f32) -> ViewportData {
        let editing = self.sketch_editing_active();
        // An edit session opening on a picked face or body: the pick has
        // done its work, and left painted it would cover what is edited.
        if editing && !self.session.plane_session_open {
            self.clear_view_selection();
        }
        self.session.plane_session_open = editing;
        let pixels_per_point = self.pixels_per_point();
        self.session.camera.set_pixels_per_point(pixels_per_point);
        self.session.camera.set_orbit_lock(editing);
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
        // Faces Ctrl added stand only beside a last face, on the body that
        // is still selected; whatever dropped that face drops them.
        let picking_faces = self.session.face_highlight.is_some()
            && self
                .session
                .last_face_hit
                .is_some_and(|(body, _)| self.session.selected_body == Some(body));
        if !picking_faces {
            self.session.earlier_faces.clear();
        }
        // Bodies a box took stand only beside a body picked whole, and only
        // while each is still drawn.
        if self.session.selected_body.is_none()
            || self.session.face_highlight.is_some()
            || !self.session.selected_edges.is_empty()
        {
            self.session.earlier_bodies.clear();
        }
        let document = &self.session.document;
        let selected = self.session.selected_body;
        self.session.earlier_bodies.retain(|body| {
            Some(*body) != selected
                && document
                    .imported_geometry(core_document::BodyId(*body))
                    .is_some()
                && document.imported_body_effective_visible(core_document::BodyId(*body))
        });

        let (sketch_meshes, region_meshes) = self.sketch_submissions(editing_feature);

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
        if let Some(side) = self.session.side_camera.as_mut() {
            let settings = &self.user_settings.camera;
            side.set_pixels_per_point(pixels_per_point);
            side.flush_pending_wheel(settings);
            side.set_scene_bounds(scene);
            side.apply_auto_clip_planes(settings);
            side.update(dt_secs, settings);
        }

        self.drive_texture_previews();
        self.drive_mesh_histories();
        self.drive_print_layout();

        let draw_style = self.user_settings.rendering.draw_style;
        let wireframe = draw_style == settings::DrawStyle::Wireframe;
        let mut imported_meshes = self.body_submissions(wireframe);

        let wb_id = self.session.active_workbench.0.clone();

        // What the active workbench shows this frame, asked once under one
        // context: its meshes in the scene (grid lines, guides), its
        // constant-size lines and text, and the chrome it fills.
        let params = self.overlay_ctx_params();
        let (shown, task) = match self
            .with_workbench_ctx(&wb_id, params, |wb, ctx| (wb.viewport(ctx), wb.task(ctx)))
        {
            Some((asked, outcome)) => {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Lifecycle);
                asked
            }
            None => Default::default(),
        };
        let core_document::ViewportOutput {
            meshes,
            overlays,
            polygons,
            images,
            marks,
            labels,
            hud,
            scalar_maps,
            status,
            clip_plane,
            view_grid,
            faded,
        } = shown;
        let mut overlay_meshes: Vec<BodySubmission> = meshes
            .into_iter()
            .enumerate()
            .map(|(i, overlay)| {
                // Overlays are asked for every frame; slot `i` keeps a stable
                // id from the pool and the maker's revision, so an unchanged
                // overlay is a GPU cache hit. The bench's id goes into the
                // revision, so a slot another bench filled is never taken
                // for this one's.
                while self.overlay_id_pool.len() <= i {
                    self.overlay_id_pool.push(Uuid::new_v4());
                }
                let vertex_colors = !overlay.mesh.colors.is_empty();
                BodySubmission {
                    scalar: None,
                    id: self.overlay_id_pool[i],
                    revision: overlay_revision(&wb_id, overlay.revision),
                    mesh: overlay.mesh,
                    color: overlay.color,
                    opacity: overlay.opacity,
                    highlight: HighlightState::None,
                    is_wireframe: overlay.wireframe,
                    // A guide is drawn, never picked.
                    pickable: false,
                    on_top: overlay.on_top,
                    edge_color: None,
                    front_only: false,
                    // A bench's mesh with its own colours keeps them see-through.
                    vertex_colors,
                    instance: None,
                    section: Default::default(),
                }
            })
            .collect();

        let mut data = ViewportData {
            overlays,
            polygons,
            images,
            marks,
            labels,
            hud,
            scalar_maps,
            status,
            task,
            editing_feature,
            clip_plane,
            view_grid,
            faded,
        };
        // Maps address the document mesh, not a texture preview's split vertices
        // or the duplicates introduced by per-face colours.
        data.scalar_maps.retain(|map| {
            let Some(body) = imported_meshes.iter_mut().find(|b| b.id == map.body.0) else {
                return false;
            };
            let Some(geometry) = self.session.document.imported_geometry(map.body) else {
                return false;
            };
            if map.values.len() != geometry.mesh.positions.len()
                || map.ramp.is_empty()
                || !map.range.iter().all(|v| v.is_finite())
                || map.range[0] >= map.range[1]
            {
                return false;
            }
            body.mesh = Arc::clone(&geometry.mesh);
            body.revision = geometry.revision;
            body.instance = None;
            body.scalar = Some(render_wgpu::ScalarField {
                values: Arc::clone(&map.values),
                revision: map.revision,
                range: map.range,
                bands: map.bands,
                ramp: Arc::clone(&map.ramp),
                under: map.under,
                over: map.over,
                none: map.none,
            });
            self.session
                .drawn_meshes
                .insert(body.id, (body.revision, geometry.revision));
            true
        });
        self.session.shown_maps.clone_from(&data.scalar_maps);
        // What the bench asks to see past draws faded, still pickable.
        for body in imported_meshes.iter_mut() {
            if data.faded.iter().any(|f| f.0 == body.id) {
                body.opacity = body.opacity.min(FADED_OPACITY);
            }
        }
        self.overlay_handles(&mut data);
        self.annotation_overlays(&mut data);

        let mut all_meshes = sketch_meshes;
        all_meshes.extend(imported_meshes);
        all_meshes.extend(region_meshes);
        // What the feature being edited adds or takes: its tool in the
        // preview colour, faces see-through and edges whole.
        let rendering = &self.user_settings.rendering;
        for (body, preview) in &self.session.previews {
            if !self.session.document.imported_body_effective_visible(*body) {
                continue;
            }
            all_meshes.push(BodySubmission {
                scalar: None,
                id: preview.id,
                revision: preview.revision,
                mesh: Arc::clone(&preview.tool),
                color: rendering.preview_color,
                opacity: rendering
                    .preview_opacity
                    .clamp(0.05, settings::MAX_SELECTION_OPACITY),
                highlight: HighlightState::None,
                is_wireframe: wireframe,
                pickable: false,
                on_top: false,
                edge_color: Some(rendering.preview_color),
                front_only: true,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }
        all_meshes.append(&mut overlay_meshes);

        // A scan's marked surface, over it while it shows.
        self.settle_marking();
        if let Some(marking) = &self.session.marking
            && !marking.drawn.indices.is_empty()
            && self
                .session
                .document
                .imported_body_effective_visible(marking.body)
        {
            let [r, g, b, _] = ui_kit::tokens::WARNING.to_normalized_gamma_f32();
            let (hi, lo) = marking.body.0.as_u64_pair();
            all_meshes.push(BodySubmission {
                scalar: None,
                id: Uuid::from_u128(MARKING_SLOT ^ u128::from(hi)),
                revision: marking.version ^ lo,
                mesh: Arc::clone(&marking.drawn),
                color: [r, g, b],
                opacity: 0.6,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        }

        self.selection_paints(&mut all_meshes);

        // The print layout's plates, each the build volume's outline.
        if let Some(shown) = &self.session.print_layout {
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.print_bed_id,
                revision: shown.key,
                mesh: Arc::clone(&shown.beds),
                color: [0.45, 0.52, 0.6],
                opacity: 1.0,
                highlight: HighlightState::None,
                is_wireframe: false,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            });
        } else if self.user_settings.printing.show_bed {
            // The printer's build volume, as twelve lines around the model.
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
                    scalar: None,
                    id: self.print_bed_id,
                    revision,
                    mesh: Arc::clone(mesh),
                    color: [0.45, 0.52, 0.6],
                    opacity: 1.0,
                    highlight: HighlightState::None,
                    is_wireframe: false,
                    pickable: false,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance: None,
                    section: Default::default(),
                });
            }
        }

        // The ground grid and the origin's planes, out of the way while an
        // edit session holds the view; a bench's own grid in the scene
        // grid's place while it hands one.
        let guides = GuideView::of(&self.session.camera);
        let rendering = &self.user_settings.rendering;
        self.frame_submission.grids.clear();
        if let Some(grid) = &data.view_grid {
            self.frame_submission
                .grids
                .push(scene_guides::bench_grid(&guides, grid));
        }
        if !editing {
            if rendering.show_grid && data.view_grid.is_none() {
                self.frame_submission
                    .grids
                    .push(self.session.grid_plane.grid(
                        &guides,
                        &self.session.camera.axis_system(),
                        rendering.grid_plane,
                        rendering,
                    ));
            }
            // Shown too while a bench asks for one of them to be picked.
            if rendering.show_origin_planes || self.bench_shows_origin_planes() {
                all_meshes.extend(self.origin_planes.bodies(
                    scene_guides::origin_plane_half(&guides),
                    self.session.hovered_base_plane,
                ));
            }
        }

        self.frame_submission.bodies = all_meshes;
        self.frame_submission.draw_edges = draw_style == settings::DrawStyle::ShadedEdges;
        // A bench's cut stands in for the toolbar's while it asks for one.
        // A bench's own plane has no wedge.
        let section_clip = self.session.section.as_ref().map(|plane| plane.clip());
        self.frame_submission.clip_plane = data.clip_plane.or(section_clip.map(|c| c.plane));
        self.frame_submission.clip_wedge = match data.clip_plane {
            Some(_) => None,
            None => section_clip.and_then(|c| c.wedge),
        };
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
    pub polygons: Vec<core_document::ScreenSpacePolygon>,
    pub images: Vec<core_document::ScreenSpaceImage>,
    pub marks: Vec<core_document::ScreenSpaceMark>,
    pub labels: Vec<core_document::ScreenSpaceLabel>,
    pub hud: Option<core_document::ViewportHud>,
    pub scalar_maps: Vec<core_document::ScalarMap>,
    pub status: Option<core_document::StatusItems>,
    pub task: Option<core_document::TaskInfo>,
    pub editing_feature: Option<core_document::FeatureId>,
    /// The plane the active bench cuts the scene at, if it does.
    pub clip_plane: Option<[f32; 4]>,
    /// The grid the active bench draws on its own plane, if it does.
    pub view_grid: Option<core_document::PlaneGrid>,
    /// Bodies the active bench asks to be drawn faded.
    pub faded: Vec<core_document::BodyId>,
}

/// How opaque a body a bench fades draws.
const FADED_OPACITY: f32 = 0.3;

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
        if let Some(shown) = &self.session.print_layout {
            for mesh in std::iter::once(&shown.beds).chain(shown.preview.iter().map(|(_, m)| m)) {
                if let Some(bounds) = mesh.bounds() {
                    add(bounds);
                }
            }
            return (lo.x <= hi.x).then_some((lo, hi));
        }
        for bounds in crate::app::view_fit::drawn_body_bounds(&self.session.document) {
            add(bounds);
        }
        for feature in features {
            if let Some(bounds) = feature.mesh.bounds() {
                add(bounds);
            }
        }
        if self.user_settings.rendering.show_origin_planes && !self.sketch_editing_active() {
            let half = scene_guides::origin_plane_half(&GuideView::of(&self.session.camera));
            add(([-half; 3], [half; 3]));
        }
        let rendering = &self.user_settings.rendering;
        if rendering.show_grid && !self.sketch_editing_active() {
            let camera = &self.session.camera;
            let (_, screen_up) = camera.view_basis();
            if let Some(ends) = self.session.grid_plane.orthographic_reach(
                &GuideView::of(camera),
                screen_up.normalize_or_zero(),
                &camera.axis_system(),
                rendering,
            ) {
                for end in ends {
                    add((end.to_array(), end.to_array()));
                }
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
                let face = crate::app::input::face_id_at_pick(
                    &geometry.mesh,
                    geometry.revision,
                    self.session.hovered_triangle,
                    point,
                )?;
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
        let body_name = self.session.document.body(body).map(|b| b.name.clone())?;
        // The face under the cursor names the feature that made it; else
        // the body is named after the last feature that shaped its solid.
        let maker = self
            .session
            .hovered_face_ref
            .as_ref()
            .and_then(|face| self.session.document.face_maker(body, face.name))
            .and_then(|id| self.session.document.get_feature_meta(id))
            .map(|n| n.name.clone());
        let feature = maker.or_else(|| {
            self.session
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.body == Some(body))
                .filter_map(|(id, n)| {
                    let info = self.registry.feature_info(n)?;
                    info.builds_solid.then_some((n.seq, *id, info.kind_label))
                })
                .max_by_key(|(seq, id, _)| (*seq, *id))
                .map(|(_, _, kind)| kind.to_string())
        });
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

    /// "x × y × z" of the selected (else active) body in the display unit.
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
        Some(ui::status_bar::size_text(
            bounds,
            self.session.document.display_unit(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_document::{BodyPlacement, Compression};

    fn part(doc: &mut Document) -> core_document::BodyId {
        let body = doc.create_body(Some("Peg".into()));
        let mesh = Arc::new(kernel_api::TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        });
        doc.set_imported_geometry(body, crate::test_support::geometry(mesh));
        doc.set_imported_brep_data(body, b"ogeom peg".to_vec(), Vec::new());
        body
    }

    #[test]
    fn copies_draw_the_mesh_they_share_and_a_mirror_or_other_body_does_not() {
        let mut doc = Document::new("t");
        let source = part(&mut doc);
        doc.set_body_placement(source, BodyPlacement::new(glam::Quat::IDENTITY, Vec3::Z));
        let a = doc.create_linked_copy(source, None).unwrap();
        let b = doc.create_linked_copy(source, None).unwrap();
        doc.set_body_placement(b, BodyPlacement::new(glam::Quat::IDENTITY, Vec3::X * 5.0));
        let (sa, sb) = (shared_mesh(&doc, a).unwrap(), shared_mesh(&doc, b).unwrap());
        assert!(Arc::ptr_eq(&sa, &sb), "one mesh for both copies");
        assert_eq!(sa.positions[1], [1.0, 0.0, 0.0], "in its own frame");
        assert!(shared_mesh(&doc, source).is_none());

        let bytes = doc.save_to_bytes(Compression::None).unwrap();
        let loaded = Document::load_from_bytes(bytes).unwrap();
        let (la, lb) = (shared_mesh(&loaded, a), shared_mesh(&loaded, b));
        assert!(
            la.zip(lb).is_some_and(|(x, y)| Arc::ptr_eq(&x, &y)),
            "after a load too"
        );
    }
}
