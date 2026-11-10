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

/// The build volume as a line body: a box of the bed's width and depth,
/// standing on Z, with the origin at the bed's corner or centre.
/// The mesh a linked copy shares with its source, in their own frame:
/// `None` for any other body, a mirrored copy, or a copy whose mesh is
/// its own.
fn shared_mesh(
    document: &Document,
    body: core_document::BodyId,
) -> Option<Arc<kernel_api::TriMesh>> {
    let entry = document.bodies().iter().find(|b| b.id == body)?;
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

/// Frames after geometry arrives at which the bench click hook requests its
/// pick (the camera has settled by then), clicks, activates the bench tool
/// named by `PRINTCAD_BENCH_TOOL`, and reports what the rebuild made of it.
const BENCH_CLICK_PICK_FRAME: u32 = 180;
const BENCH_CLICK_FRAME: u32 = 191;
const BENCH_TOOL_FRAME: u32 = 192;
const BENCH_REPORT_FRAME: u32 = 300;
/// Idle frames `PRINTCAD_BENCH_EDIT` waits for before it opens its feature.
const BENCH_EDIT_IDLE_FRAMES: u32 = 30;

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
        if let Some(path) = std::env::var_os("PRINTCAD_BENCH_PICTURE") {
            let written =
                crate::app::animation::png_of(picture.width, picture.height, &picture.rgba)
                    .and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string()));
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
            || self.radial_press.is_some()
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
            || std::env::var_os("PRINTCAD_BENCH_ORBIT").is_some()
            || std::env::var_os("PRINTCAD_EXIT_AFTER_MS").is_some()
            || std::env::var_os("PRINTCAD_BENCH_SPIN").is_some()
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
            self.keep_session_for_next_run();
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
            // Without a hook, the command line's files or the last run's.
            if crate::launch_screen() == crate::ui::Screen::Start {
                self.reopen_at_start();
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
        // With no body of that name, the feature named exactly so, as a
        // click on its tree row would, once the document has bodies.
        let named_feature = |document: &core_document::Document, which: &str| {
            document
                .feature_tree()
                .all_nodes()
                .find(|(_, node)| node.name == which)
                .map(|(id, _)| *id)
        };
        if !self.bench_select_fired
            && let Ok(which) = std::env::var("PRINTCAD_BENCH_SELECT")
            && !self.session.document.bodies().is_empty()
            && self.kernel_worker.in_flight() == 0
            && let Some(feature) = named_feature(&self.session.document, &which)
        {
            self.bench_select_fired = true;
            self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature));
            tracing::info!(target: "printcad.frame", "bench selected feature `{which}`");
        }
        // Dev/bench hook: `PRINTCAD_BENCH_ISOLATE=<name>` isolates the view
        // to the first body whose name contains the text, once it has
        // geometry and while nothing is isolated.
        if self.session.document.isolation().is_none()
            && let Ok(which) = std::env::var("PRINTCAD_BENCH_ISOLATE")
            && let Some(body) = self
                .session
                .document
                .bodies()
                .iter()
                .find(|b| b.name.contains(&which))
                .map(|b| b.id)
            && self.session.document.imported_geometry(body).is_some()
            && !std::mem::replace(&mut self.bench_isolate_fired, true)
        {
            self.session.document.set_isolation(Some(vec![body]));
        }

        // Dev/bench hook: `PRINTCAD_BENCH_NOTE=<number>` arms the note tool
        // and opens the card on that note once its body has geometry, as a
        // click on it would, so a capture shows the card.
        if !self.bench_note_fired
            && let Some(number) = std::env::var("PRINTCAD_BENCH_NOTE")
                .ok()
                .and_then(|n| n.parse::<u32>().ok())
            && let Some((body, note)) = self.session.document.note_by_number(number)
            && self.session.document.imported_geometry(body).is_some()
        {
            let id = note.id;
            self.bench_note_fired = true;
            self.session.note_tool = true;
            self.open_note_card(id);
        }

        // Dev/bench hook: `PRINTCAD_BENCH_EDIT=<name>` opens the first
        // feature of that name for editing, as a double click on its tree
        // row would (a sketch opens its editing session), once the kernel
        // has been idle for a while with the document open, so a capture
        // can show any document's sketch.
        let idle =
            self.kernel_worker.in_flight() == 0 && !self.session.document.bodies().is_empty();
        self.bench_edit_idle = if idle { self.bench_edit_idle + 1 } else { 0 };
        if !self.bench_edit_fired
            && let Ok(name) = std::env::var("PRINTCAD_BENCH_EDIT")
            && self.bench_edit_idle >= BENCH_EDIT_IDLE_FRAMES
        {
            self.bench_edit_fired = true;
            // The earliest in the history when several share the name.
            let found = self
                .session
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, node)| node.name == name)
                .min_by_key(|(_, node)| node.seq)
                .map(|(id, _)| *id);
            match found {
                Some(id) => {
                    self.apply_tree_activation(crate::ui::TreeItemId::Feature(id));
                    tracing::info!(target: "printcad.frame", "bench editing `{name}`");
                }
                None => tracing::warn!(target: "printcad.frame", "bench edit: no feature `{name}`"),
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_CHAT=<message>` sends the message,
        // its `@` mentions taken from the list as a pick would take them,
        // in a chat with no agent behind it, once the kernel has been idle
        // a while (and `PRINTCAD_BENCH_SELECT` has selected), and the
        // assistant opens on it with `@` typed, so a capture shows chips and
        // the list.
        if !self.bench_chat_fired
            && self.bench_edit_idle >= BENCH_EDIT_IDLE_FRAMES
            && (self.bench_select_fired || std::env::var_os("PRINTCAD_BENCH_SELECT").is_none())
            && let Ok(message) = std::env::var("PRINTCAD_BENCH_CHAT")
        {
            self.bench_chat_fired = true;
            self.bench_chat(message);
        }

        // Dev/bench hook: once `PRINTCAD_BENCH_EDIT` has opened its feature
        // (or with no such feature named) and the camera has settled,
        // `PRINTCAD_BENCH_POINTER=<fx>,<fy>` holds the pointer at that
        // fraction of the viewport, with the tool `PRINTCAD_BENCH_TOOL`
        // names in hand, so a capture shows a tool's cues, or the view's
        // hover, where the pointer is.
        if (self.bench_edit_fired || std::env::var_os("PRINTCAD_BENCH_EDIT").is_none())
            && !self.session.camera.is_animating()
            && std::env::var_os("PRINTCAD_BENCH_CLICK").is_none()
            && let Ok(spec) = std::env::var("PRINTCAD_BENCH_POINTER")
            && let Some((fx, fy)) = spec
                .split_once(',')
                .and_then(|(a, b)| Some((a.parse::<f32>().ok()?, b.parse::<f32>().ok()?)))
        {
            let tool = std::env::var("PRINTCAD_BENCH_TOOL")
                .ok()
                .map(|tool| core_document::renamed::command(&tool).into_owned());
            if let Some(tool) = &tool {
                self.session.active_tool.active_ids.insert(tool.clone());
            }
            let vp = self.session.camera.viewport_info();
            let at = (fx * vp.2 as f32, fy * vp.3 as f32);
            self.cursor_in_viewport = Some(at);
            if !self.sketch_editing_active() {
                self.session.hovered_feature = self.feature_under_cursor();
                self.session.hovered_part = self.part_under_cursor();
            }
            let wb_id = self.active_workbench_id();
            self.call_workbench_input(
                &wb_id,
                &core_document::WorkbenchInputEvent::MouseMove { viewport_pos: at },
                tool.as_deref(),
            );
        }

        // Dev/bench hook: `PRINTCAD_BENCH_PICTURE=<path>` takes one picture
        // of the view once the scene has settled and writes it there.
        // `PRINTCAD_BENCH_WINDOW=<path>` does the same for the whole
        // window, the panels and menus with it.
        const PICTURE_SETTLE_FRAMES: u32 = 60;
        let scene = std::env::var_os("PRINTCAD_BENCH_PICTURE").is_some();
        let window = std::env::var_os("PRINTCAD_BENCH_WINDOW").is_some();
        // Settling waits out a camera that is still turning to its view,
        // which a fast build would otherwise catch half way, and a scripted
        // click and its tool, an edit or a chat until they have had their
        // turn.
        // `PRINTCAD_BENCH_UI`'s steps play once the scene has settled, and
        // the picture waits for them.
        let settled = self.kernel_worker.in_flight() == 0
            && !self.session.camera.is_animating()
            && (self.bench_edit_fired || std::env::var_os("PRINTCAD_BENCH_EDIT").is_none())
            && self.bench_edit_idle >= BENCH_EDIT_IDLE_FRAMES;
        let mut playing = false;
        let mut view_click = None;
        if let Some(gfx) = self.gfx.as_mut() {
            if settled {
                gfx.ui_layer.arm_bench_ui();
            }
            view_click = gfx.ui_layer.take_bench_view_click();
            playing = gfx.ui_layer.bench_ui_pending();
        }
        if let Some(at) = view_click {
            self.bench_view_click(at);
        }
        if playing {
            self.redraw_needed = true;
        }
        let clicking = playing
            || std::env::var_os("PRINTCAD_BENCH_CLICK").is_some()
                && self.bench_click_frames <= BENCH_REPORT_FRAME
            || std::env::var_os("PRINTCAD_BENCH_EDIT").is_some() && !self.bench_edit_fired
            || std::env::var_os("PRINTCAD_BENCH_CHAT").is_some() && !self.bench_chat_fired;
        if (scene || window)
            && self.bench_picture_frames <= PICTURE_SETTLE_FRAMES
            && self.kernel_worker.in_flight() == 0
            && !self.session.camera.is_animating()
            && !clicking
        {
            self.bench_picture_frames += 1;
            self.redraw_needed = true;
            if self.bench_picture_frames > PICTURE_SETTLE_FRAMES
                && let Some(gfx) = self.gfx.as_mut()
            {
                if scene {
                    gfx.renderer
                        .request_capture(render_wgpu::CaptureKind::Scene);
                }
                if window {
                    gfx.renderer
                        .request_capture(render_wgpu::CaptureKind::Window);
                }
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_WORKBENCH=<bench id>` switches to
        // that bench once the document has a body, as the switcher would,
        // so a capture shows its toolbar and `PRINTCAD_BENCH_TOOL` reaches
        // its tools.
        if !self.bench_workbench_fired
            && let Ok(id) = std::env::var("PRINTCAD_BENCH_WORKBENCH")
            && !self.session.document.bodies().is_empty()
        {
            self.bench_workbench_fired = true;
            let target = crate::WorkbenchId::from(id.as_str());
            if self.registry.descriptor(&target).is_some() {
                self.switch_workbench_for_flow(target);
            } else {
                tracing::warn!(target: "printcad.frame", "no bench `{id}` to switch to");
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_TASK=appearance|placement|history|
        // texture|print_layout|array|note|mark` opens that task of the
        // application's on the first body (its first feature, for history;
        // its first note, or a new one; the marking of a scan) once it has
        // geometry, as its menu entry would, so a capture shows the task
        // panel.
        if !self.bench_task_fired
            && let Ok(which) = std::env::var("PRINTCAD_BENCH_TASK")
            && let Some(body) = self.session.document.bodies().first().map(|b| b.id)
            && self.session.document.imported_geometry(body).is_some()
            && let Some(gfx) = self.gfx.as_mut()
        {
            self.bench_task_fired = true;
            let first = self
                .session
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.body == Some(body))
                .min_by_key(|(id, n)| (n.seq, **id))
                .map(|(id, _)| *id);
            let open = match which.as_str() {
                "placement" => Some(ui::OpenTask::Placement(body)),
                "history" => first.map(ui::OpenTask::History),
                "texture" => Some(ui::OpenTask::Texture(body, None)),
                "print_layout" => Some(ui::OpenTask::PrintLayout),
                "array" => Some(ui::OpenTask::Array(body)),
                "note" => Some(match self.session.document.body_notes(body).first() {
                    Some(note) => ui::OpenTask::EditNote(note.id),
                    None => ui::OpenTask::AddNote(body, None),
                }),
                "mark" => Some(ui::OpenTask::Mark(body)),
                _ => Some(ui::OpenTask::Appearance(body, None)),
            };
            if let Some(open) = open {
                gfx.ui_layer
                    .open_task(&self.session.document, open, self.session.tab);
            }
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
        // body to become a solid, once, as the tree's menu would; `=refine`
        // then asks for every converted solid's refine once it lands.
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

        if !self.bench_refine_fired
            && std::env::var("PRINTCAD_BENCH_CONVERT").as_deref() == Ok("refine")
        {
            let faceted: Vec<_> = self
                .session
                .document
                .bodies()
                .iter()
                .map(|b| b.id)
                .filter(|b| self.session.document.can_refine(*b))
                .collect();
            if !faceted.is_empty() {
                self.bench_refine_fired = true;
                self.apply_ui_commands(vec![ui::UiCommand::RefineShapes(faceted)], event_loop);
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
                        "bench click at ({cx}, {cy}) of {:?}: body {:?} at {:?}, triangle {:?}, edge {:?}, face {:?}",
                        (vp.2, vp.3),
                        self.session.hovered_body,
                        self.session.hovered_world_pos,
                        self.session.hovered_triangle,
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
                        let tool = core_document::renamed::command(&tool).into_owned();
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

        // Dev/bench hook: `PRINTCAD_BENCH_SECTION=<aim>[@<offset>][!][^<way>][~<body>]` turns
        // the clipping plane on once the first body has geometry (after
        // `PRINTCAD_BENCH_CLICK`'s click, when there is one), aimed as the
        // clip pill would: `x`, `y`, `z`, `view`, `face` (the selected flat
        // face, else waiting for a click), `points:x,y,z;x,y,z;x,y,z` or
        // `saved:<name>` (a section the document keeps),
        // moved `offset` mm along its normal, `!` keeping the other side,
        // `^0` or `^1` adding a wedge facing that way, the body of that
        // name left whole.
        if !self.bench_section_fired
            && let Ok(spec) = std::env::var("PRINTCAD_BENCH_SECTION")
            && self
                .session
                .document
                .bodies()
                .first()
                .is_some_and(|b| self.session.document.imported_geometry(b.id).is_some())
            && (std::env::var_os("PRINTCAD_BENCH_CLICK").is_none()
                || self.bench_click_frames > BENCH_CLICK_FRAME)
        {
            self.bench_section_fired = true;
            self.bench_section(&spec);
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
        // The faces selected in the view, as their body's mesh
        // numbers them, in the order picked: what the Appearance
        // and texture tasks take.
        let picked_faces: Vec<(core_document::BodyId, u32)> = self
            .session
            .selected_body
            .filter(|_| self.session.face_highlight.is_some())
            .map(core_document::BodyId)
            .and_then(|body| {
                let mesh = &self.session.document.imported_geometry(body)?.mesh;
                Some(
                    self.selected_face_refs()
                        .into_iter()
                        .filter_map(|face| {
                            Some((body, crate::app::input::face_id_at(mesh, face.point)?))
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
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
        };

        let commands;
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
                        radial: self.radial.as_ref().map(|radial| {
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
                self.session.print_layout_shown = ui_result.print_layout_open;
                if self.session.mark_arm != ui_result.mark_arm {
                    self.session.mark_arm = ui_result.mark_arm;
                    self.session.mark_gesture = None;
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
                        let path = std::env::var_os("PRINTCAD_BENCH_WINDOW").unwrap_or_default();
                        let written = crate::app::animation::png_of(
                            picture.width,
                            picture.height,
                            &picture.rgba,
                        )
                        .and_then(|png| std::fs::write(&path, png).map_err(|e| e.to_string()));
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
        let cursor = self.cursor_in_viewport?;
        let vp = self.session.camera.viewport_info();
        let (origin, dir) = core_document::runtime::viewport_to_ray(
            self.session.camera.view_projection(),
            (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            cursor,
        )?;
        let (origin, dir) = (glam::Vec3::from_array(origin), glam::Vec3::from_array(dir));
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

        let mut region_meshes: Vec<BodySubmission> = Vec::new();
        let mut regions_seen: Vec<(core_document::FeatureId, usize)> = Vec::new();
        let picked_parts = self.session.selected_parts();
        let hovered_curve = self
            .session
            .hovered_part
            .clone()
            .filter(|(_, part)| matches!(part, core_document::FeaturePart::Edge(_)));
        let sketch_meshes: Vec<BodySubmission> = self
            .registry
            .passive_geometries(&self.session.document, editing_feature)
            .into_iter()
            .map(|(feature_id, geometry)| {
                // Match the in-edit overlay palette: its geometry colour, the
                // amber hover, and its selected colour, the selection's
                // blue. Color is baked directly so the tint is unmistakable
                // even on hairline geometry.
                let is_active = self.session.active_document_object == Some(feature_id)
                    || self.session.tree_selection
                        == Some(crate::ui::TreeItemId::Feature(feature_id));
                // Parts of it picked on their own leave the rest plain.
                let parts: &[core_document::FeaturePart] =
                    if self.session.active_document_object == Some(feature_id) {
                        &picked_parts
                    } else {
                        &[]
                    };
                let is_selected = is_active && parts.is_empty();
                // A curve under the cursor lights on its own, not its whole
                // sketch.
                let curve_lit = hovered_curve
                    .as_ref()
                    .is_some_and(|(f, _)| *f == feature_id);
                let is_hovered = (self.session.hovered_feature == Some(feature_id) && !curve_lit)
                    || self.session.tree_hovered
                        == Some(crate::ui::TreeItemId::Feature(feature_id));
                let palette = core_document::SketchPalette::default();
                let plain = match geometry.tint {
                    core_document::PassiveTint::Plain => palette.geometry,
                    core_document::PassiveTint::External => palette.external,
                };
                let color = if is_selected {
                    palette.selected
                } else if is_hovered {
                    HOVER_PAINT
                } else {
                    plain
                };
                // The tint participates in the cache revision so hover /
                // selection transitions actually re-upload the color.
                let state_bits = (is_selected as u64) | ((is_hovered as u64) << 1);
                // What it encloses, region by region, shaded see-through
                // behind its lines and meshed once per revision; a region
                // picked or under the cursor lit in its paint.
                let strongest = geometry
                    .regions
                    .iter()
                    .map(|r| r.opacity)
                    .fold(0.0_f32, f32::max);
                for (index, region) in geometry.regions.iter().enumerate() {
                    let key = (feature_id, index);
                    regions_seen.push(key);
                    let fresh = self
                        .region_meshes
                        .get(&key)
                        .is_some_and(|(revision, _)| *revision == geometry.revision);
                    if !fresh {
                        use kernel_api::KernelQueries;
                        let mesh = kernel_ogeom::QUERIES
                            .profile_mesh(&region.profile)
                            .unwrap_or_default();
                        self.region_meshes
                            .insert(key, (geometry.revision, Arc::new(mesh)));
                    }
                    let Some((_, mesh)) = self.region_meshes.get(&key) else {
                        continue;
                    };
                    if mesh.indices.is_empty() {
                        continue;
                    }
                    let part = region.part.as_ref();
                    let picked = part.is_some_and(|p| parts.contains(p));
                    let under_cursor = part.is_some_and(|p| {
                        self.session
                            .hovered_part
                            .as_ref()
                            .is_some_and(|(f, h)| *f == feature_id && h == p)
                    });
                    let (region_color, lit, state) = if picked {
                        (palette.selected, true, 3)
                    } else if under_cursor {
                        (HOVER_PAINT, true, 4)
                    } else {
                        (color, false, state_bits)
                    };
                    region_meshes.push(BodySubmission {
                        scalar: None,
                        id: Uuid::from_u128(
                            feature_id.0.as_u128() ^ REGION_ID_SALT ^ ((index as u128) << 64),
                        ),
                        revision: geometry.revision ^ (state << 61),
                        mesh: Arc::clone(mesh),
                        color: region_color,
                        highlight: HighlightState::None,
                        is_wireframe: false,
                        opacity: if lit {
                            (strongest * LIT_REGION_GAIN).min(1.0)
                        } else {
                            region.opacity
                        },
                        pickable: false,
                        on_top: false,
                        edge_color: None,
                        front_only: false,
                        vertex_colors: false,
                        instance: None,
                        section: Default::default(),
                    });
                }
                BodySubmission {
                    scalar: None,
                    id: feature_id.0,
                    revision: geometry.revision ^ (state_bits << 62),
                    mesh: Arc::new(geometry.mesh),
                    color,
                    highlight: HighlightState::None,
                    is_wireframe: false,
                    opacity: 1.0,
                    pickable: true,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance: None,
                    section: Default::default(),
                }
            })
            .collect();
        // The curves picked on their own, over the feature's lines in the
        // selection paint, and the curve under the cursor in the hover
        // paint.
        let mut sketch_meshes = sketch_meshes;
        let part_lines = |feature_id: core_document::FeatureId,
                          part: &core_document::FeaturePart,
                          id: Uuid,
                          color: [f32; 3]| {
            let lines = self
                .registry
                .part_lines(&self.session.document, feature_id, part)?;
            let revision = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for p in &lines.positions {
                    p.map(f32::to_bits).hash(&mut h);
                }
                h.finish()
            };
            Some(BodySubmission {
                scalar: None,
                id,
                revision,
                mesh: Arc::new(lines),
                color,
                highlight: HighlightState::None,
                is_wireframe: false,
                opacity: 1.0,
                pickable: false,
                on_top: false,
                edge_color: None,
                front_only: false,
                vertex_colors: false,
                instance: None,
                section: Default::default(),
            })
        };
        let palette = core_document::SketchPalette::default();
        if let Some(feature_id) = self.session.active_document_object {
            for (index, part) in picked_parts.iter().enumerate() {
                let id = Uuid::from_u128(
                    feature_id.0.as_u128() ^ PART_LINES_ID_SALT ^ ((index as u128) << 64),
                );
                sketch_meshes.extend(part_lines(feature_id, part, id, palette.selected));
            }
        }
        if let Some((feature_id, part)) = &hovered_curve {
            let picked = self.session.active_document_object == Some(*feature_id)
                && picked_parts.contains(part);
            if !picked {
                let id = Uuid::from_u128(feature_id.0.as_u128() ^ HOVERED_LINES_ID_SALT);
                sketch_meshes.extend(part_lines(*feature_id, part, id, HOVER_PAINT));
            }
        }

        self.region_meshes.retain(|id, _| regions_seen.contains(id));

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

        // Imported geometry (e.g. STEP files) becomes regular renderable bodies.
        // The body id from the document is reused so picking/selection stays
        // stable, and the document's revision counter is forwarded to the
        // renderer so panning/orbiting never re-uploads the static mesh.
        let draw_style = self.user_settings.rendering.draw_style;
        let wireframe = draw_style == settings::DrawStyle::Wireframe;
        let scene_clip = self.scene_clip();
        let section = self.session.section.clone();
        let mut imported_meshes: Vec<BodySubmission> = self
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
                let entry = self
                    .session
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.id == *body_id);
                let chosen = entry.and_then(|b| b.display);
                let pickable = !entry.is_some_and(|b| b.unselectable);
                let face_colors = entry.map(|b| b.face_colors.as_slice()).unwrap_or(&[]);
                let use_vertex_albedo = chosen.is_none()
                    && geometry.mesh.colors.len() == geometry.mesh.positions.len()
                    && !geometry.mesh.colors.is_empty();
                let base_color = match chosen {
                    Some(display) => display.color,
                    None if use_vertex_albedo => [1.0, 1.0, 1.0],
                    None => core_document::BodyDisplay::default().color,
                };
                let opacity = chosen.map(|d| d.opacity.clamp(0.05, 1.0)).unwrap_or(1.0);
                // A textured body draws its textured mesh, the last one
                // made while a newer is being made. It is in the body's
                // own frame, drawn moved where the body sits.
                let pressed = self
                    .session
                    .textured
                    .get(body_id)
                    .and_then(|p| p.made.as_ref());
                let textured = pressed.is_some();
                let (shown, shown_revision) = match pressed {
                    Some((key, mesh)) => (Arc::clone(mesh), *key),
                    None => (Arc::clone(&geometry.mesh), geometry.revision),
                };
                // A mesh body shaded otherwise than as meshed draws its
                // triangles reshaded, made once per change.
                let shading = entry.map(|b| b.shading).unwrap_or_default();
                let (shown, shown_revision) = if textured
                    || shading == core_document::Shading::AsMeshed
                    || !self.session.document.is_mesh_body(*body_id)
                {
                    (shown, shown_revision)
                } else {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    shown_revision.hash(&mut hasher);
                    format!("{shading:?}").hash(&mut hasher);
                    let key = hasher.finish();
                    let mesh = match self.session.shaded.get(body_id) {
                        Some((k, mesh)) if *k == key => Arc::clone(mesh),
                        _ => {
                            let crease = match shading {
                                core_document::Shading::Smooth { crease_deg } => Some(crease_deg),
                                _ => None,
                            };
                            let mesh = Arc::new(crate::app::shading::reshade(&shown, crease));
                            self.session
                                .shaded
                                .insert(*body_id, (key, Arc::clone(&mesh)));
                            mesh
                        }
                    };
                    (mesh, key)
                };
                // Faces coloured on their own: the mesh with those colours
                // in it, the rest in the body's, made once per change.
                let (mesh, revision, color) = if face_colors.is_empty() {
                    (shown, shown_revision, base_color)
                } else {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    shown_revision.hash(&mut hasher);
                    for c in face_colors {
                        (c.name, c.index, c.color.map(f32::to_bits)).hash(&mut hasher);
                    }
                    chosen.map(|d| d.color.map(f32::to_bits)).hash(&mut hasher);
                    let key = hasher.finish();
                    let cached = self
                        .session
                        .face_colored
                        .get(body_id)
                        .filter(|(k, _)| *k == key)
                        .map(|(_, m)| Arc::clone(m));
                    let mesh = cached.unwrap_or_else(|| {
                        // A body's own colours stay where no chosen colour
                        // covers them.
                        let base = if chosen.is_some() {
                            let mut plain = (*shown).clone();
                            plain.colors.clear();
                            core_document::mesh_with_face_colors(&plain, face_colors, base_color)
                        } else {
                            core_document::mesh_with_face_colors(&shown, face_colors, base_color)
                        };
                        let mesh = Arc::new(base);
                        self.session
                            .face_colored
                            .insert(*body_id, (key, Arc::clone(&mesh)));
                        mesh
                    });
                    (mesh, key, [1.0, 1.0, 1.0])
                };
                // A linked copy drawn as its source's mesh is (no face
                // coloured, no texture, not mirrored) draws the mesh it
                // shares with the source, moved where it sits: the GPU
                // holds that mesh once for every copy. A textured body
                // draws its pressed mesh alike, which a source and its
                // copies share when their textures are the same.
                let shared = if textured {
                    Some(Arc::clone(&mesh))
                } else if face_colors.is_empty() {
                    shared_mesh(&self.session.document, *body_id)
                } else {
                    None
                };
                let (mesh, revision, instance) = match shared {
                    Some(shared) => {
                        let ptr = Arc::as_ptr(&shared) as usize;
                        let key = self
                            .session
                            .shared_meshes
                            .entry(ptr)
                            .or_insert_with(|| (Arc::downgrade(&shared), Uuid::new_v4()))
                            .1;
                        let transform = self.session.document.body_placement(*body_id).matrix();
                        (
                            shared,
                            0,
                            Some(render_wgpu::MeshInstance { key, transform }),
                        )
                    }
                    None => (mesh, revision, None),
                };
                // Under the clipping plane, an open mesh shows its inside
                // as the shell it is; a closed one is capped. A body the
                // toolbar's section leaves whole is neither.
                let uncut = scene_clip.is_some()
                    && crate::camera::section::clip_for(scene_clip, section.as_ref(), *body_id)
                        .is_none();
                let open = !uncut && {
                    let closed = match self.session.closed.get(body_id) {
                        Some((at, closed)) if *at == geometry.revision => *closed,
                        _ => {
                            let closed = crate::app::scans::closes(&geometry.mesh);
                            self.session
                                .closed
                                .insert(*body_id, (geometry.revision, closed));
                            closed
                        }
                    };
                    !closed
                };
                BodySubmission {
                    scalar: None,
                    id: body_id.0,
                    revision,
                    mesh,
                    color,
                    opacity,
                    highlight,
                    is_wireframe: wireframe,
                    pickable,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance,
                    section: render_wgpu::SectionLook {
                        open,
                        hatch: None,
                        uncut,
                    },
                }
            })
            .collect();
        // Where several closed bodies are cut, each cap hatches at its own
        // angle, so nested parts read apart.
        if self.frame_submission.clip_plane.is_some() {
            let capped: Vec<usize> = (0..imported_meshes.len())
                .filter(|i| {
                    let look = imported_meshes[*i].section;
                    !look.open && !look.uncut
                })
                .collect();
            if capped.len() > 1 {
                const ANGLES: [f32; 4] = [0.785, -0.785, 0.262, -1.309];
                for (n, i) in capped.into_iter().enumerate() {
                    imported_meshes[i].section.hatch = Some(ANGLES[n % ANGLES.len()]);
                }
            }
        }
        // A shared mesh no copy draws any more is let go.
        self.session
            .shared_meshes
            .retain(|_, (mesh, _)| mesh.strong_count() > 0);
        // What a pick's triangle numbers: each body drawn with the
        // document's mesh, or a copy of it in its order with faces
        // coloured; a textured body's triangles are split.
        self.session.drawn_meshes.clear();
        for submitted in &imported_meshes {
            let body = core_document::BodyId(submitted.id);
            let textured = self
                .session
                .textured
                .get(&body)
                .is_some_and(|p| p.made.is_some());
            if let Some(geometry) = self.session.document.imported_geometry(body)
                && !textured
            {
                self.session
                    .drawn_meshes
                    .insert(submitted.id, (submitted.revision, geometry.revision));
            }
        }
        // While the Print layout task is open the scene is the layout: the
        // copies on the bed, drawn but not picked, the bodies left where
        // they are.
        if let Some(shown) = &self.session.print_layout {
            let document = &self.session.document;
            imported_meshes = shown
                .layout
                .pieces
                .iter()
                .zip(&shown.preview)
                .map(|(piece, (id, mesh))| BodySubmission {
                    scalar: None,
                    id: *id,
                    revision: shown.key,
                    mesh: Arc::clone(mesh),
                    color: document
                        .bodies()
                        .iter()
                        .find(|b| b.id == piece.body)
                        .and_then(|b| b.display)
                        .unwrap_or_default()
                        .color,
                    opacity: 1.0,
                    highlight: HighlightState::None,
                    is_wireframe: wireframe,
                    pickable: false,
                    on_top: false,
                    edge_color: None,
                    front_only: false,
                    vertex_colors: false,
                    instance: None,
                    section: Default::default(),
                })
                .collect();
        }

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
                let vertex_colors = !overlay.mesh.colors.is_empty();
                BodySubmission {
                    scalar: None,
                    id: self.overlay_id_pool[i],
                    revision: overlay
                        .revision
                        .unwrap_or_else(|| hash_trimesh(&overlay.mesh)),
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

        // Screen-space overlays + labels from the active workbench
        // (constant-thickness lines and constant-size text).
        let params = self.overlay_ctx_params();
        let mut data = match self.with_workbench_ctx(&wb_id, params, |wb, ctx| ViewportData {
            overlays: wb.get_screen_space_overlays(ctx, ctx.active_document_object),
            polygons: wb.get_screen_space_polygons(ctx, ctx.active_document_object),
            images: wb.get_screen_space_images(ctx, ctx.active_document_object),
            marks: wb.get_screen_space_marks(ctx, ctx.active_document_object),
            labels: wb.get_screen_space_labels(ctx, ctx.active_document_object),
            hud: wb.viewport_hud(ctx),
            scalar_maps: wb.scalar_maps(ctx),
            status: wb.status_items(ctx),
            task: wb.task(ctx),
            editing_feature: wb.editing_feature(),
            clip_plane: wb.clip_plane(ctx),
            view_grid: wb.view_grid(ctx),
            faded: wb.faded_bodies(ctx),
        }) {
            Some((data, outcome)) => {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Lifecycle);
                data
            }
            None => ViewportData::default(),
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
        let screen_space_labels = &mut data.labels;

        // The selection box being dragged, in the colour the sketcher's
        // box draws in.
        if let Some(drag) = &self.box_select {
            data.overlays
                .extend(drag.outline(core_document::SketchPalette::default().preview));
        }
        data.overlays
            .extend(self.mark_outline(core_document::SketchPalette::default().preview));

        // The clipping plane's handle, an arrow along its normal.
        if let Some(handle) = self.section_handle() {
            let hot = self.section_handle_hot(&handle);
            let (lines, marks, label) = handle.drawn(hot, self.session.document.display_unit());
            data.overlays.extend(lines);
            data.marks.extend(marks);
            screen_space_labels.push(label);
        }

        // The array task's spacing handle on its row's first copy.
        if let Some(handle) = self.array_handle() {
            let hot = self.array_handle_hot(&handle);
            let (lines, marks, label) = handle.drawn(hot, self.session.document.display_unit());
            data.overlays.extend(lines);
            data.marks.extend(marks);
            screen_space_labels.push(label);
        }

        // The measurement in progress: its picks pinned and numbered, the
        // path joining them and the readout of the last two, drawn over the
        // scene; a pick on a body the split view's side pane shows is
        // pinned there.
        if let Some(picks) = &self.session.measure {
            let unit = self.measure_unit();
            let color = HOVER_PAINT;
            let vp = self.session.camera.viewport_info();
            let vp = (vp.0 as u32, vp.1 as u32, vp.2, vp.3);
            let side = self.side_pane_view(vp);
            let px: Vec<Option<(f32, f32)>> = picks
                .iter()
                .map(|p| match (&side, p.body()) {
                    (Some(side), Some(body)) if side.bodies.contains(&body.0) => {
                        core_document::runtime::world_to_viewport(side.view_proj, vp, p.point())
                            .map(|(x, y)| (x + side.offset.0, y + side.offset.1))
                    }
                    _ => self
                        .session
                        .camera
                        .world_to_viewport(Vec3::from_array(p.point())),
                })
                .collect();
            let [r, g, b, _] = ui_kit::tokens::WARNING.to_normalized_gamma_f32();
            let (marks, labels) = core_document::numbered_pins(
                px.iter()
                    .enumerate()
                    .filter_map(|(i, at)| at.map(|(x, y)| ([x, y], i + 1))),
                [r, g, b],
                self.pixels_per_point(),
            );
            data.marks.extend(marks);
            screen_space_labels.extend(labels);
            let mut lines = crate::app::measure::readout(picks, unit);
            // The kernel's least distance between the last two picks, with
            // the closest points joined.
            if let Some(gap) = &self.session.measure_gap {
                match (&gap.found, gap.in_world()) {
                    (_, Some((distance, a, b))) => {
                        lines.insert(
                            0,
                            format!(
                                "Least distance {}",
                                crate::app::measure::length(distance, unit)
                            ),
                        );
                        let camera = &self.session.camera;
                        if let (Some(a), Some(b)) =
                            (camera.world_to_viewport(a), camera.world_to_viewport(b))
                        {
                            let mut line = core_document::ScreenSpaceOverlay::new(
                                [a.0, a.1],
                                [b.0, b.1],
                                color,
                                2.0,
                            );
                            line.dash = Some((5.0, 3.0));
                            data.overlays.push(line);
                        }
                    }
                    (None, _) => lines.insert(0, "Least distance …".to_string()),
                    (Some(Err(why)), _) => {
                        lines.insert(0, format!("Least distance unknown: {why}"));
                    }
                    _ => {}
                }
            }
            for pair in px.windows(2) {
                if let [Some(pa), Some(pb)] = pair {
                    data.overlays.push(core_document::ScreenSpaceOverlay::new(
                        [pa.0, pa.1],
                        [pb.0, pb.1],
                        color,
                        1.5,
                    ));
                }
            }
            // The readout between the last two picks, one line a row.
            let anchor = match px.as_slice() {
                [.., Some(pa), Some(pb)] => Some(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0)),
                [.., last] => *last,
                [] => None,
            };
            if let Some(anchor) = anchor {
                for (i, line) in lines.into_iter().enumerate() {
                    screen_space_labels.push(
                        core_document::ScreenSpaceLabel::new(
                            [anchor.0 + 14.0, anchor.1 - 12.0 + 20.0 * i as f32],
                            line,
                            color,
                            12.0,
                        )
                        .pill(),
                    );
                }
            }
            if let Some((x, y)) = self.cursor_in_viewport {
                let prompt = match picks.len() {
                    0 => "Measure: pick a point, an edge, a face or a datum",
                    1 => "Measure: pick a second one to measure between",
                    _ => "Measure: pick another to go on, or empty space to start over",
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
        // Each paint's submission by the body it is drawn for, so it draws
        // in that body's pane of a split view.
        let mut overlay_bodies: Vec<(Uuid, Uuid)> = Vec::new();
        // The faces Ctrl added before the last, each in a slot of its own.
        for (i, picked) in self.session.earlier_faces.iter().enumerate() {
            let face = &picked.highlight;
            let (hi, lo) = face.body.as_u64_pair();
            let id = Uuid::from_u128(self.face_highlight_id.as_u128() ^ (i as u128 + 1));
            overlay_bodies.push((id, face.body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id,
                revision: face.revision ^ hi ^ lo ^ (u64::from(face.face.unwrap_or(0)) << 32),
                mesh: Arc::clone(&face.mesh),
                color: paint,
                opacity,
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
        if let Some(face) = &self.session.face_highlight {
            overlay_bodies.push((self.face_highlight_id, face.body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.face_highlight_id,
                revision: face.revision,
                mesh: Arc::clone(&face.mesh),
                color: paint,
                opacity,
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
        } else if let Some(geometry) = self
            .session
            .selected_body
            // Picked edges are the selection then, drawn as lines below.
            .filter(|_| self.session.selected_edges.is_empty())
            // A body showing a feature's preview is not painted over: the
            // preview is what it shows.
            .filter(|id| {
                !self
                    .session
                    .previews
                    .contains_key(&core_document::BodyId(*id))
            })
            .and_then(|id| {
                self.session
                    .document
                    .imported_geometry(core_document::BodyId(id))
            })
        {
            // One slot serves every body; the body's id in the revision
            // keeps two bodies at the same revision from sharing buffers.
            let selected = self.session.selected_body.unwrap_or_default();
            overlay_bodies.push((self.body_highlight_id, selected));
            let (hi, lo) = selected.as_u64_pair();
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.body_highlight_id,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: paint,
                opacity,
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

        // The bodies a box took beside the last, each in a slot of its own.
        for (i, body) in self.session.earlier_bodies.iter().enumerate() {
            let id = core_document::BodyId(*body);
            if self.session.previews.contains_key(&id) {
                continue;
            }
            let Some(geometry) = self.session.document.imported_geometry(id) else {
                continue;
            };
            let (hi, lo) = body.as_u64_pair();
            let slot = Uuid::from_u128(self.body_highlight_id.as_u128() ^ (i as u128 + 1));
            overlay_bodies.push((slot, *body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: slot,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: paint,
                opacity,
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

        // The body a tree row under the pointer stands for, translucent in
        // the hover paint, unless it is selected already.
        if let Some(body) = self.tree_hovered_body()
            && self.session.selected_body != Some(body.0)
            && !self.session.previews.contains_key(&body)
            && let Some(geometry) = self.session.document.imported_geometry(body)
        {
            let (hi, lo) = body.0.as_u64_pair();
            overlay_bodies.push((self.tree_hover_id, body.0));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.tree_hover_id,
                revision: geometry.revision ^ hi ^ lo,
                mesh: Arc::clone(&geometry.mesh),
                color: HOVER_PAINT,
                opacity: opacity * 0.5,
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

        // The hovered face, translucent in the hover paint, unless it is the
        // selected face already.
        if let Some(hover) = &self.session.hovered_face
            && self.session.hovered_edge.is_none()
            && self.session.pick_filter.faces()
            && !self
                .session
                .face_highlight
                .iter()
                .chain(self.session.earlier_faces.iter().map(|p| &p.highlight))
                .any(|f| f.body == hover.body && f.face == Some(hover.face))
        {
            let (hi, lo) = hover.body.as_u64_pair();
            overlay_bodies.push((self.face_hover_id, hover.body));
            all_meshes.push(BodySubmission {
                scalar: None,
                id: self.face_hover_id,
                revision: hover.revision ^ hi ^ lo ^ (u64::from(hover.face) << 32),
                mesh: Arc::clone(&hover.mesh),
                color: HOVER_PAINT,
                opacity: opacity * 0.5,
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
            overlay_bodies.push((self.edge_hover_id, hit.body));
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
                overlay_bodies.push((id, *body));
                all_meshes.push(submission);
            }
        }
        // A paint over a body the section leaves whole is left whole too.
        if self.frame_submission.clip_plane.is_some() {
            for (slot, body) in &overlay_bodies {
                if self.clip_of(core_document::BodyId(*body)).is_none()
                    && let Some(paint) = all_meshes.iter_mut().find(|m| m.id == *slot)
                {
                    paint.section.uncut = true;
                }
            }
        }
        self.session.overlay_bodies = overlay_bodies;

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
    /// Dev/bench hook: a body with a small constrained sketch, opened for
    /// editing, so the sketcher can be exercised without clicking; `pad`
    /// pads it and opens the pad for editing, `pocket` pockets the pad's
    /// top too and opens the pocket.
    /// A left click in the viewport at fraction `at` of it, as the
    /// viewport's own input hands it to the active bench with the tool in
    /// hand.
    fn bench_view_click(&mut self, at: egui::Pos2) {
        let vp = self.session.camera.viewport_info();
        let viewport_pos = (at.x * vp.2 as f32, at.y * vp.3 as f32);
        self.cursor_in_viewport = Some(viewport_pos);
        let tool = self.session.active_tool.active_ids.iter().next().cloned();
        let wb_id = self.active_workbench_id();
        let button = core_document::MouseButton::Left;
        for event in [
            core_document::WorkbenchInputEvent::MouseMove { viewport_pos },
            core_document::WorkbenchInputEvent::MousePress {
                button,
                viewport_pos,
            },
            core_document::WorkbenchInputEvent::MouseRelease {
                button,
                viewport_pos,
            },
        ] {
            self.call_workbench_input(&wb_id, &event, tool.as_deref());
        }
        self.redraw_needed = true;
    }

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
                self.apply_tree_activation(crate::ui::TreeItemId::Feature(handles.feature));
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
        let body_name = self
            .session
            .document
            .bodies()
            .iter()
            .find(|b| b.id == body)
            .map(|b| b.name.clone())?;
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
