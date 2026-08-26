//! Per-frame work: pacing, scene submission assembly, UI run, render, pick.

use std::sync::Arc;
use std::time::{Duration, Instant};

use core_document::WorkbenchFeature;
use glam::Vec3;
use render_vk::{
    BodySubmission, GpuLight, HighlightState, LightingData, RenderBackend,
    ViewportRect as RenderViewportRect,
};
use settings::UserSettings;
use uuid::Uuid;
use winit::event_loop::{ActiveEventLoop, ControlFlow};

use crate::log_panel as app_log;
use crate::orientation_cube::OrientationCubeInput;
use crate::{ui, Document, PrintCadApp};

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

impl PrintCadApp {
    /// Body of `about_to_wait`: pace the frame, drain worker channels,
    /// assemble the scene, run the UI, render, read back the pick, and
    /// apply this frame's UI commands.
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

        let mut new_body_requested = false;

        // Pull any STEP imports that the kernel worker finished off the
        // queue before we build this frame's submission, so freshly imported
        // bodies show up immediately and the import log lines stay tied to
        // the frame they actually became visible in. Has to happen before
        // we take a mutable borrow on `self.renderer` below.
        self.drain_kernel_responses();
        self.drain_document_open_responses();
        self.drain_document_save_responses();
        self.drive_part_recompute();

        if self.gfx.is_none() {
            return;
        }

        // Update camera animation and assemble this frame's scene submission
        // before the UI/render block takes its borrows on `gfx`.
        let (screen_space_overlays, screen_space_labels) = self.build_scene_submission(dt_secs);

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

                let ui_result = ui_layer.run(
                    window,
                    ui::UiFrameInputs {
                        active_tool: self.active_tool.clone(),
                        active_workbench: self.active_workbench.clone(),
                        settings: &mut self.user_settings,
                        document: &mut self.document,
                        registry: &mut self.registry,
                        orientation_input: Some(&orientation_input),
                        fps: self.current_fps,
                        gpu_name: self.gpu_name.as_deref(),
                        gpus: &self.available_gpus,
                        hovered_point: self.hovered_world_pos,
                        pivot_screen_pos,
                        axis_system: self.camera.axis_system(),
                        tree_selection: self.tree_selection,
                        active_document_object: self.active_document_object,
                        selected_body_id: self.active_body_id,
                        screen_space_overlays: &screen_space_overlays,
                        screen_space_labels: &screen_space_labels,
                        pending_imports: self.kernel_worker.in_flight(),
                        pending_document_open: self.document_open_in_flight
                            + self.document_save_in_flight,
                        kernel_status: self.kernel_worker.status(),
                        kernel_progress: self.kernel_worker.progress(),
                        kernel_cancellable: self.kernel_worker.is_cancellable(),
                        document_saving: self.document_save_in_flight > 0,
                        step_import_pending: self.step_import_pending.as_mut(),
                    },
                );
                self.frame_submission.egui = Some(ui_result.submission);
                self.active_tool = ui_result.active_tool;
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

            window.request_redraw();

            if let Err(err) = renderer.render(&self.frame_submission) {
                app_log::error(format!("Render failure: {err}"));
                event_loop.exit();
                return;
            }

            // Retrieve pick result from GPU picking (processed during render)
            let pick_result = renderer.latest_pick_result();
            self.hovered_body = pick_result.body_id;
            self.hovered_world_pos = pick_result.world_position;
        }

        // Apply this frame's UI actions now that the renderer borrow is over.
        self.apply_ui_commands(commands, new_body_requested, event_loop);

        // Cut an undo boundary at frame end when no drag is in progress so
        // an entire drag interaction coalesces into one step.
        if self.mouse_buttons_down == 0 {
            self.undo.note(&self.document);
        }
    }

    /// Update the camera and assemble this frame's [`FrameSubmission`]
    /// (sketch tessellations, imported bodies, workbench overlay meshes).
    /// Returns the workbench's screen-space overlays, which are drawn via
    /// egui rather than the 3D pass.
    /// True while the sketch workbench is editing a sketch. Gates the
    /// camera's out-of-plane rotation so the view stays planar.
    pub(crate) fn sketch_editing_active(&self) -> bool {
        self.active_workbench.0.as_str() == "wb.sketch"
            && self
                .active_document_object
                .and_then(|id| self.document.get_feature_meta(id))
                .map(|n| n.workbench_id.as_str() == "wb.sketch")
                .unwrap_or(false)
    }

    fn build_scene_submission(
        &mut self,
        dt_secs: f32,
    ) -> (
        Vec<core_document::ScreenSpaceOverlay>,
        Vec<core_document::ScreenSpaceLabel>,
    ) {
        self.camera.set_orbit_lock(self.sketch_editing_active());
        self.camera.flush_pending_wheel(&self.user_settings.camera);
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
        let editing_sketch = if self.active_workbench.0.as_str() == "wb.sketch" {
            self.active_document_object
        } else {
            None
        };
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

                let mesh = wb_sketch::render::sketch_to_mesh(
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
                // While a single FACE is the selection, the body itself is
                // not tinted — only the face overlay below highlights.
                let face_only = self
                    .face_highlight
                    .as_ref()
                    .map(|f| f.body == body_id.0)
                    .unwrap_or(false);
                let is_selected = self.selected_body == Some(body_id.0) && !face_only;
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
                    highlight: HighlightState::None,
                    is_wireframe,
                }
            })
            .collect();

        // Screen-space overlays + labels from the active workbench
        // (constant-thickness lines and constant-size text).
        let params = self.overlay_ctx_params();
        let (screen_space_overlays, screen_space_labels) = self
            .with_workbench_ctx(&wb_id, params, |wb, ctx| {
                (
                    wb.get_screen_space_overlays(ctx, ctx.active_document_object),
                    wb.get_screen_space_labels(ctx, ctx.active_document_object),
                )
            })
            .map(|(pair, _outcome)| pair)
            .unwrap_or_default();

        // Combine sketch meshes, imported geometry, and overlay meshes.
        let mut all_meshes = sketch_meshes;
        all_meshes.extend(imported_meshes);
        all_meshes.append(&mut overlay_meshes);

        // Selected-face highlight: the coplanar sub-mesh, slightly lifted
        // off the surface, in the selection green.
        if let Some(face) = &self.face_highlight {
            all_meshes.push(BodySubmission {
                id: self.face_highlight_id,
                revision: face.revision,
                mesh: Arc::clone(&face.mesh),
                color: [0.35, 0.95, 0.45],
                highlight: HighlightState::None,
                is_wireframe: false,
            });
        }

        self.frame_submission.bodies = all_meshes;
        self.frame_submission.view_proj = self.camera.view_projection();
        self.frame_submission.camera_pos = self.camera.position();
        self.frame_submission.lighting = lighting_data_from_settings(&self.user_settings);

        (screen_space_overlays, screen_space_labels)
    }
}
