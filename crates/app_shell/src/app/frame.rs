//! Per-frame work: pacing, scene submission assembly, UI run, render, pick.

use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

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

/// How long after the last input frames keep coming, so egui's reactions
/// and the pick readbacks land.
const INPUT_TAIL: Duration = Duration::from_millis(150);

/// Mixed into a feature's id for the id of the region it shades, so the
/// two never share a renderer cache slot.
const REGION_ID_SALT: u128 = 0x5245_4749_4f4e_5f53_4841_4445_0000_0001;

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
    /// settles. The wake gate and the end-of-frame scheduler share this: a
    /// source listed in only one of them either burns CPU or sleeps through
    /// its own completion.
    fn async_work_pending(&self) -> bool {
        self.texture_previews_pending()
            || self.picture.is_some()
            || self
                .gfx
                .as_ref()
                .is_some_and(|gfx| gfx.renderer.capture_pending())
            || self.kernel_worker.in_flight() > 0
            || self.any_tab_busy()
            || self.file_dialog_rx.is_some()
            || self.export_rx.is_some()
            || !self.nav_device.motion().is_idle()
            || self.script_thread.busy()
            || self.registry.any_busy()
            || self.package_work.busy()
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
        // When the loop must wake for the next autosave, idle or not.
        let autosave_at = self.next_autosave();
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
            let animating = self.session.camera.is_animating()
                || std::env::var_os("PRINTCAD_BENCH_ORBIT").is_some()
                || std::env::var_os("PRINTCAD_EXIT_AFTER_MS").is_some()
                || std::env::var_os("PRINTCAD_BENCH_SPIN").is_some();
            let autosave_due = autosave_at.is_some_and(|at| at <= now);
            if !(self.redraw_needed
                || input_active
                || self.async_work_pending()
                || animating
                || autosave_due
                || self.pending_ui_repaint.is_zero())
            {
                // Asleep, the loop still wakes for the next autosave.
                event_loop.set_control_flow(match autosave_at {
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

        // Dev/bench hook: `PRINTCAD_BENCH_PICTURE=<path>` takes one picture
        // of the view once the scene has settled and writes it there.
        const PICTURE_SETTLE_FRAMES: u32 = 60;
        if std::env::var_os("PRINTCAD_BENCH_PICTURE").is_some()
            && self.bench_picture_frames <= PICTURE_SETTLE_FRAMES
            && self.kernel_worker.in_flight() == 0
        {
            self.bench_picture_frames += 1;
            self.redraw_needed = true;
            if self.bench_picture_frames > PICTURE_SETTLE_FRAMES
                && let Some(gfx) = self.gfx.as_mut()
            {
                gfx.renderer.request_capture();
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
        // texture|print_layout` opens that task of the application's on the
        // first body (its first feature, for history) once it has geometry,
        // as its menu entry would, so a capture shows the task panel.
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

        let server_status = self.session.server.status();
        let ui_repaint_delay;

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
            && document_imported_aabb(&self.session.document).is_some()
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
        crate::platform::set_unsaved(
            std::iter::once(&self.session)
                .chain(self.tabs.iter().filter_map(|slot| slot.parked.as_ref()))
                .any(|session| session.document.metadata().dirty()),
        );
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
            status: status_items,
            task,
            editing_feature,
            clip_plane: _,
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
        let physical = self.panel_physical();
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
            bought_kinds: self.registry.bought_kinds(),
        };

        let commands;
        // The cursor is on the scene itself, not on a menu or card over it.
        let over_scene;

        {
            let tabs = self.tab_infos();
            let unsaved_question = self.unsaved_question();
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
                        active_document_object: self.session.active_document_object,
                        editing_feature,
                        viewport_hud,
                        status_items,
                        task,
                        hover_card,
                        picks,
                        dimensions,
                        physical,
                        field_of_view_deg: self.session.camera.field_of_view_deg(),
                        section: self.session.section,
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
                        reveal_body: self.session.reveal_body.take(),
                        viewport_menu: self.session.viewport_menu.clone(),
                        picked_faces,
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
                        assistant_attention: std::mem::take(&mut self.assistant_attention),
                        print_layout: self.session.print_layout.as_ref().map(|s| &s.layout),
                    },
                );
                self.frame_phase_accum.0 += ui_started.elapsed().as_secs_f32() * 1000.0;
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
            if let Some(picture) = renderer.take_capture() {
                self.picture = Some(picture);
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
            // Field-level reads, not `async_work_pending()`: a `&self`
            // method call cannot coexist with the live `gfx` borrow, and
            // this must be frame-END truth: a kernel job submitted during
            // this frame has to keep the loop awake. Mirror the helper.
            let work_pending = renderer.capture_pending()
                || self.session.textured.values().any(|p| p.making.is_some())
                || self.picture.is_some()
                || self.kernel_worker.in_flight() > 0
                || crate::app::tabs::tabs_busy(&self.session, &self.tabs)
                || self.file_dialog_rx.is_some()
                || self.export_rx.is_some()
                || !self.nav_device.motion().is_idle()
                || self.script_thread.busy()
                || self.registry.any_busy()
                || self.package_work.busy();
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
                event_loop.set_control_flow(match autosave_at {
                    Some(at) => ControlFlow::WaitUntil(at),
                    None => ControlFlow::Wait,
                });
            }

            // Retrieve pick result from GPU picking (processed during render).
            // The pick answers for where the cursor last was on the scene; a
            // menu or card opened under a still cursor, or the cursor gone
            // off the view, leaves nothing hovered, rather than the last
            // answer coming back each frame.
            over_scene = self.cursor_in_viewport.is_some() && !ui_layer.pointer_over_floating_ui();
            let pick_result = renderer.latest_pick_result();
            self.session.hovered_body = pick_result.body_id.filter(|_| over_scene);
            self.session.hovered_world_pos = pick_result.world_position.filter(|_| over_scene);
            self.session.pick_depths = pick_result.depth_window;
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

        let mut region_meshes: Vec<BodySubmission> = Vec::new();
        let mut regions_seen: Vec<core_document::FeatureId> = Vec::new();
        let sketch_meshes: Vec<BodySubmission> = self
            .registry
            .passive_geometries(&self.session.document, editing_feature)
            .into_iter()
            .map(|(feature_id, geometry)| {
                // Match the in-edit overlay palette: its geometry colour, the
                // amber hover, and its selected colour, the selection's
                // blue. Color is baked directly so the tint is unmistakable
                // even on hairline geometry.
                let is_selected = self.session.active_document_object == Some(feature_id)
                    || self.session.tree_selection
                        == Some(crate::ui::TreeItemId::Feature(feature_id));
                let is_hovered = self.session.hovered_feature == Some(feature_id)
                    || self.session.tree_hovered
                        == Some(crate::ui::TreeItemId::Feature(feature_id));
                let palette = core_document::SketchPalette::default();
                let color = if is_selected {
                    palette.selected
                } else if is_hovered {
                    HOVER_PAINT
                } else {
                    match geometry.tint {
                        core_document::PassiveTint::Plain => palette.geometry,
                        core_document::PassiveTint::External => palette.external,
                    }
                };
                // The tint participates in the cache revision so hover /
                // selection transitions actually re-upload the color.
                let state_bits = (is_selected as u64) | ((is_hovered as u64) << 1);
                // What it encloses, shaded see-through behind its lines and
                // meshed once per revision.
                if let Some(region) = &geometry.region {
                    regions_seen.push(feature_id);
                    let fresh = self
                        .region_meshes
                        .get(&feature_id)
                        .is_some_and(|(revision, _)| *revision == geometry.revision);
                    if !fresh {
                        use kernel_api::KernelQueries;
                        let mesh = kernel_ogeom::QUERIES
                            .profile_mesh(&region.profile)
                            .unwrap_or_default();
                        self.region_meshes
                            .insert(feature_id, (geometry.revision, Arc::new(mesh)));
                    }
                    if let Some((_, mesh)) = self.region_meshes.get(&feature_id)
                        && !mesh.indices.is_empty()
                    {
                        region_meshes.push(BodySubmission {
                            id: Uuid::from_u128(feature_id.0.as_u128() ^ REGION_ID_SALT),
                            revision: geometry.revision ^ (state_bits << 62),
                            mesh: Arc::clone(mesh),
                            color,
                            highlight: HighlightState::None,
                            is_wireframe: false,
                            opacity: region.opacity,
                            pickable: false,
                            on_top: false,
                            edge_color: None,
                            front_only: false,
                        });
                    }
                }
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
                    edge_color: None,
                    front_only: false,
                }
            })
            .collect();

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

        self.drive_texture_previews();
        self.drive_print_layout();

        // Imported geometry (e.g. STEP files) becomes regular renderable bodies.
        // The body id from the document is reused so picking/selection stays
        // stable, and the document's revision counter is forwarded to the
        // renderer so panning/orbiting never re-uploads the static mesh.
        let draw_style = self.user_settings.rendering.draw_style;
        let wireframe = draw_style == settings::DrawStyle::Wireframe;
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
                // made while a newer is being made.
                let (shown, shown_revision) = match self
                    .session
                    .textured
                    .get(body_id)
                    .and_then(|p| p.made.as_ref())
                {
                    Some((key, mesh)) => (Arc::clone(mesh), *key),
                    None => (Arc::clone(&geometry.mesh), geometry.revision),
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
                BodySubmission {
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
                }
            })
            .collect();
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
                    edge_color: None,
                    front_only: false,
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
            status: wb.status_items(ctx),
            task: wb.task(ctx),
            editing_feature: wb.editing_feature(),
            clip_plane: wb.clip_plane(ctx),
            faded: wb.faded_bodies(ctx),
        }) {
            Some((data, outcome)) => {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Lifecycle);
                data
            }
            None => ViewportData::default(),
        };
        // What the bench asks to see past draws faded, still pickable.
        for body in imported_meshes.iter_mut() {
            if data.faded.iter().any(|f| f.0 == body.id) {
                body.opacity = body.opacity.min(FADED_OPACITY);
            }
        }
        let screen_space_labels = &mut data.labels;

        // The measurement in progress: its points, the line between them
        // and the distance, drawn over the scene.
        if let Some(picks) = &self.session.measure {
            let unit = self.session.document.display_unit();
            let color = HOVER_PAINT;
            let px: Vec<(f32, f32)> = picks
                .iter()
                .filter_map(|p| {
                    self.session
                        .camera
                        .world_to_viewport(Vec3::from_array(p.point()))
                })
                .collect();
            for (x, y) in &px {
                data.marks.push(core_document::ScreenSpaceMark::crosshair(
                    [*x, *y],
                    8.0,
                    color,
                ));
            }
            let lines = match picks.as_slice() {
                [a, b] => crate::app::measure::describe_pair(a, b, unit),
                [one] => one.describe(unit),
                _ => Vec::new(),
            };
            if let [pa, pb] = px.as_slice() {
                data.overlays.push(core_document::ScreenSpaceOverlay::new(
                    [pa.0, pa.1],
                    [pb.0, pb.1],
                    color,
                    1.5,
                ));
            }
            // The readout beside the last pick, one line a row.
            if let Some((x, y)) = px.last().copied() {
                let anchor = match px.as_slice() {
                    [pa, pb] => ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0),
                    _ => (x, y),
                };
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
            if picks.len() < 2
                && let Some((x, y)) = self.cursor_in_viewport
            {
                let prompt = if picks.is_empty() {
                    "Measure: pick a point, an edge or a face"
                } else {
                    "Measure: pick a second one to measure between"
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
            });
        }
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
        // The faces Ctrl added before the last, each in a slot of its own.
        for (i, picked) in self.session.earlier_faces.iter().enumerate() {
            let face = &picked.highlight;
            let (hi, lo) = face.body.as_u64_pair();
            all_meshes.push(BodySubmission {
                id: Uuid::from_u128(self.face_highlight_id.as_u128() ^ (i as u128 + 1)),
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
            });
        }
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
                edge_color: None,
                front_only: false,
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
                edge_color: None,
                front_only: false,
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
            all_meshes.push(BodySubmission {
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
                edge_color: None,
                front_only: false,
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

        // The print layout's plates, each the build volume's outline.
        if let Some(shown) = &self.session.print_layout {
            all_meshes.push(BodySubmission {
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
                });
            }
        }

        // The ground grid and the origin's planes, out of the way while an
        // edit session draws its own plane's grid.
        let guides = GuideView::of(&self.session.camera);
        let rendering = &self.user_settings.rendering;
        self.frame_submission.grids.clear();
        if !editing {
            if rendering.show_grid {
                self.frame_submission.grids.push(scene_guides::ground_grid(
                    &guides,
                    &self.session.camera.axis_system(),
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
        self.frame_submission.clip_plane = data
            .clip_plane
            .or_else(|| self.session.section.map(|plane| plane.equation()));
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
    pub status: Option<core_document::StatusItems>,
    pub task: Option<core_document::TaskInfo>,
    pub editing_feature: Option<core_document::FeatureId>,
    /// The plane the active bench cuts the scene at, if it does.
    pub clip_plane: Option<[f32; 4]>,
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
        if self.user_settings.rendering.show_origin_planes && !self.sketch_editing_active() {
            let half = scene_guides::origin_plane_half(&GuideView::of(&self.session.camera));
            add(([-half; 3], [half; 3]));
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
