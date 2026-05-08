//! FreeCAD / Coin-style CAD camera navigation for printCAD (`camera_system.md`).
//!
//! Invariants worth preserving:
//! - **`focal_distance`** measured along **`forward`** (eye→focal ray), not spherical radius quirks.
//! - **Pan** translates `eye`; focal point stays `eye + forward * focal_distance` (orientation fixed).
//! - **Perspective zoom** dolls along **forward** so the pivot stays fixed unless zoom-to-cursor corrects.
//! Vulkan Y-flip, unproject parity, and large-coordinate pitfalls are summarized in **`camera_system.md`** (floating origin, clipping).

mod animate;
mod auto_clip;
mod math;
mod ops;
pub(crate) mod state;
mod zoom_cursor;

use animate::CameraTween;
use axes::{AxisPreset, AxisSystem};
use glam::{DVec3, Mat3, Quat, Vec2, Vec3};
use settings::CameraSettings;
use state::{canonical_quat_to_world, CadCameraState};
use tracing::{debug, trace};
use winit::event::{MouseButton, MouseScrollDelta, WindowEvent};
use crate::orientation_cube::{CameraSnapView, RotateAxis, RotateDelta};

pub struct CameraController {
    pub(crate) state: CadCameraState,
    axes: AxisSystem,
    axis_preset: AxisPreset,
    tween: CameraTween,
    scene_aabb: Option<(Vec3, Vec3)>,
    pending_wheel_lines: f32,
    last_cursor_viewport: Option<Vec2>,
    last_cursor_vp_for_drag: Option<Vec2>,
    lmb_anchor_vp: Vec2,
    /// True once LMB drag crosses `click_drag_threshold_px` starting from empty viewport.
    lmb_dragging_scene: bool,
    rmb_dragging_scene: bool,
    lmb_dragging_roll: bool,
    lmb_was_down_scene: bool,
    allow_lmb_orbit: bool,
}

impl CameraController {
    pub fn new(settings: &CameraSettings, initial_viewport: (u32, u32)) -> Self {
        Self {
            state: CadCameraState::new(settings, initial_viewport),
            axes: AxisSystem::from(settings.axis_preset),
            axis_preset: settings.axis_preset,
            tween: CameraTween::None,
            scene_aabb: None,
            pending_wheel_lines: 0.0,
            last_cursor_viewport: None,
            last_cursor_vp_for_drag: None,
            lmb_anchor_vp: Vec2::ZERO,
            lmb_dragging_scene: false,
            rmb_dragging_scene: false,
            lmb_dragging_roll: false,
            lmb_was_down_scene: false,
            allow_lmb_orbit: false,
        }
    }

    pub(crate) fn cancel_animation(&mut self) {
        self.tween.cancel();
    }

    /// Begin / update pointer drag modes. Gesture style: LMB orbit empty, RMB pan, LMB+RMB tilt.
    ///
    /// `allow_lmb_orbit` typically `hovered_body.is_none()` captured at LMB press from last pick.
    pub fn on_viewport_pointer(
        &mut self,
        event: &WindowEvent,
        settings: &CameraSettings,
        allow_lmb_orbit: Option<bool>,
    ) -> CameraPointerResult {
        match event {
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: winit::event::ElementState::Pressed,
                ..
            } => {
                self.cancel_animation();
                self.lmb_was_down_scene = true;
                self.allow_lmb_orbit = allow_lmb_orbit.unwrap_or(false);
                self.lmb_dragging_scene = false;
                self.lmb_dragging_roll = false;
                if let Some(p) = self.last_cursor_viewport {
                    self.lmb_anchor_vp = p;
                    self.last_cursor_vp_for_drag = Some(p);
                }
                CameraPointerResult::Redraw
            }
            WindowEvent::MouseInput {
                button: MouseButton::Right,
                state: winit::event::ElementState::Pressed,
                ..
            } => {
                self.cancel_animation();
                self.rmb_dragging_scene = true;
                self.last_cursor_vp_for_drag = self.last_cursor_viewport;
                CameraPointerResult::Redraw
            }
            WindowEvent::MouseInput {
                button: MouseButton::Middle,
                state: winit::event::ElementState::Pressed,
                ..
            } => CameraPointerResult::None,
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: winit::event::ElementState::Released,
                ..
            } => {
                let should_maybe_select = !self.lmb_dragging_scene && !self.lmb_dragging_roll;
                let did_orbit_drag = self.lmb_dragging_scene;

                self.last_cursor_vp_for_drag = None;
                self.lmb_dragging_roll = false;
                self.lmb_was_down_scene = false;
                self.allow_lmb_orbit = false;
                self.lmb_dragging_scene = false;

                if should_maybe_select && self.last_cursor_viewport.is_some() {
                    return CameraPointerResult::LmbReleasedMaybeSelect;
                }
                if did_orbit_drag {
                    CameraPointerResult::Redraw
                } else {
                    CameraPointerResult::None
                }
            }
            WindowEvent::MouseInput {
                button: MouseButton::Right,
                state: winit::event::ElementState::Released,
                ..
            } => {
                self.last_cursor_vp_for_drag = None;
                self.rmb_dragging_scene = false;
                self.lmb_dragging_roll = false;
                CameraPointerResult::None
            }
            WindowEvent::CursorMoved { .. } => {
                let Some(cur) = self.last_cursor_viewport else {
                    return CameraPointerResult::None;
                };
                let Some(last) = self.last_cursor_vp_for_drag else {
                    return CameraPointerResult::None;
                };
                let delta = cur - last;
                if delta.length_squared() < 1e-12 {
                    return CameraPointerResult::None;
                }

                self.cancel_animation();

                let both_mb = self.lmb_was_down_scene && self.rmb_dragging_scene;
                if both_mb {
                    self.lmb_dragging_roll = true;
                    ops::roll_pixels(&mut self.state, &self.axes, delta.x);
                    self.last_cursor_vp_for_drag = Some(cur);
                    return CameraPointerResult::Redraw;
                }

                if self.rmb_dragging_scene && !self.lmb_was_down_scene {
                    ops::pan_pixels(&mut self.state, &self.axes, delta, settings);
                    self.last_cursor_vp_for_drag = Some(cur);
                    self.state.clip_dirty = true;
                    return CameraPointerResult::Redraw;
                }

                if self.lmb_was_down_scene && self.allow_lmb_orbit {
                    let thresh_sq =
                        settings.click_drag_threshold_px * settings.click_drag_threshold_px;
                    if (cur - self.lmb_anchor_vp).length_squared() >= thresh_sq {
                        self.lmb_dragging_scene = true;
                    }
                    if self.lmb_dragging_scene {
                        ops::orbit_pixels(&mut self.state, &self.axes, delta, settings);
                        self.last_cursor_vp_for_drag = Some(cur);
                        self.state.clip_dirty = true;
                        return CameraPointerResult::Redraw;
                    }
                }

                CameraPointerResult::None
            }
            WindowEvent::MouseWheel { delta, .. } => {
                match delta {
                    MouseScrollDelta::LineDelta(_, y) => self.pending_wheel_lines += *y,
                    MouseScrollDelta::PixelDelta(pos) => {
                        self.pending_wheel_lines += pos.y as f32 / 120.0;
                    }
                }
                CameraPointerResult::Redraw
            }
            _ => CameraPointerResult::None,
        }
    }

    /// Must be called from `PhysicalPosition`, converted to viewport-local pixels.
    pub fn set_cursor_viewport(&mut self, pos: Option<Vec2>) {
        self.last_cursor_viewport = pos;
    }

    pub fn flush_pending_wheel(&mut self, settings: &CameraSettings) {
        let lines = self.pending_wheel_lines;
        self.pending_wheel_lines = 0.0;
        if lines.abs() <= 1e-6 {
            return;
        }
        self.cancel_animation();
        zoom_cursor::apply_zoom_wheels(
            &mut self.state,
            &self.axes,
            self.last_cursor_viewport,
            lines,
            settings,
        );
    }

    /// Middle mouse — uses current pick world position supplied by caller.
    pub fn on_mmb_pivot_pick(&mut self, world_hit: Option<Vec3>, settings: &CameraSettings) {
        if let Some(hit) = world_hit {
            if ops::set_pivot_world_hit(&mut self.state, &self.axes, hit, settings) {
                self.state.clip_dirty = true;
                self.cancel_animation();
            }
        }
    }

    pub fn pivot_from_key_h(&mut self, settings: &CameraSettings) -> bool {
        let Some(vp) = self.last_cursor_viewport else {
            return false;
        };
        self.cancel_animation();
        ops::set_pivot_focal_plane_cursor(&mut self.state, &self.axes, vp, settings)
    }

    pub fn update(&mut self, dt_secs: f32, settings: &CameraSettings) -> bool {
        let mut out = (self.state.eye, self.state.orientation, self.state.focal_distance);
        let had = self.tween.tick(dt_secs, &mut out);
        if had {
            self.state.eye = out.0;
            self.state.orientation = out.1.normalize();
            self.state.focal_distance = out.2;
            self.state.clamp_focal_distance(settings);
            self.state.clip_dirty = true;
            true
        } else {
            false
        }
    }

    pub fn update_viewport(&mut self, origin: (u32, u32), size: (u32, u32)) {
        self.state.viewport_origin = (origin.0 as f32, origin.1 as f32);
        self.state.viewport_size = size;
        self.state.clip_dirty = true;
    }

    pub fn view_projection(&self) -> [[f32; 4]; 4] {
        self.state
            .view_projection(&self.axes)
            .to_cols_array_2d()
    }

    pub fn viewport_info(&self) -> (f32, f32, u32, u32) {
        (
            self.state.viewport_origin.0,
            self.state.viewport_origin.1,
            self.state.viewport_size.0,
            self.state.viewport_size.1,
        )
    }

    pub fn world_to_screen(&self, world_pos: Vec3) -> Option<(f32, f32)> {
        let (w, h) = self.state.viewport_size;
        if w == 0 || h == 0 {
            return None;
        }
        let view_proj = self.state.view_projection(&self.axes);

        let clip = view_proj * world_pos.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        let screen_x = (ndc.x + 1.0) * 0.5 * w as f32 + self.state.viewport_origin.0;
        let screen_y = (ndc.y + 1.0) * 0.5 * h as f32 + self.state.viewport_origin.1;
        Some((screen_x, screen_y))
    }

    pub fn focal_point_world(&self) -> Vec3 {
        self.state.focal_point_vec3(&self.axes)
    }

    /// Red crosshair at orbit focal point: only while LMB orbit / LMB+RMB roll / view tween.
    pub fn rotation_pivot_marker_visible(&self) -> bool {
        self.lmb_dragging_scene
            || self.lmb_dragging_roll
            || matches!(self.tween, CameraTween::Running { .. })
    }

    pub fn viewport_to_plane(
        &self,
        viewport_x: f32,
        viewport_y: f32,
        plane_origin: Vec3,
        plane_normal: Vec3,
    ) -> Option<Vec3> {
        let (w, h) = self.state.viewport_size;
        if w == 0 || h == 0 {
            return None;
        }
        let ndc_x = (viewport_x / w as f32) * 2.0 - 1.0;
        let ndc_y = (viewport_y / h as f32) * 2.0 - 1.0;
        let vp = self.state.view_projection(&self.axes);
        let inv = vp.inverse();
        let near_clip = inv * glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far_clip = inv * glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        if near_clip.w.abs() < 1e-12 || far_clip.w.abs() < 1e-12 {
            return None;
        }
        let near_w = near_clip.truncate() / near_clip.w;
        let far_w = far_clip.truncate() / far_clip.w;
        let ray_dir = (far_w - near_w).normalize();
        let ray_origin = self.position_vec();

        let n = plane_normal.normalize();
        let denom = ray_dir.dot(n);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (plane_origin - ray_origin).dot(n) / denom;
        if t < 0.0 {
            return None;
        }
        Some(ray_origin + ray_dir * t)
    }

    pub fn reset_to_fit(
        &mut self,
        center: Vec3,
        radius: f32,
        zoom_limit_aabb: Option<(Vec3, Vec3)>,
        settings: &CameraSettings,
    ) {
        self.cancel_animation();
        self.scene_aabb = zoom_limit_aabb;
        ops::fit_sphere(
            &mut self.state,
            &self.axes,
            center,
            radius,
            zoom_limit_aabb,
            settings,
        );
        self.after_scene_or_settings_touch(settings);
    }

    pub fn clear_scene_zoom_constraint(&mut self) {
        self.scene_aabb = None;
        self.state.clip_dirty = true;
    }

    pub fn clamp_focal_to_settings(&mut self, settings: &CameraSettings) {
        self.state.clamp_focal_distance(settings);
    }

    pub fn apply_auto_clip_planes(&mut self, settings: &CameraSettings) {
        auto_clip::update_auto_clip(&mut self.state, &self.axes, self.scene_aabb, settings);
        trace!(
            target: "printcad.camera",
            proj = ?self.state.projection,
            focal_mm = self.state.focal_distance,
            ortho_height_mm = self.state.ortho_height,
            height_angle_deg = self.state.height_angle_rad.to_degrees(),
            near = self.state.near_plane,
            far = self.state.far_plane,
            clip_auto = settings.auto_near_far,
            "after auto_near_far"
        );
    }

    fn after_scene_or_settings_touch(&mut self, settings: &CameraSettings) {
        self.state.clamp_focal_distance(settings);
        self.state.clip_dirty = true;
    }

    pub fn sync_with_settings(&mut self, settings: &CameraSettings) {
        if self.axis_preset != settings.axis_preset {
            self.axis_preset = settings.axis_preset;
            self.axes = AxisSystem::from(self.axis_preset);
        }
        self.state.height_angle_rad = (settings.fov_degrees as f64).to_radians();
        self.state
            .ortho_height = settings.ortho_height_mm as f64;

        let target_proj = settings.projection;
        if target_proj != self.state.projection {
            self.state
                .set_projection_preserve_framing(target_proj, &self.axes);
        }

        self.state.clamp_focal_distance(settings);
        self.after_scene_or_settings_touch(settings);

        debug!(
            target: "printcad.camera",
            proj = ?self.state.projection,
            focal_mm = self.state.focal_distance,
            ortho_height_mm = self.state.ortho_height,
            height_angle_deg = self.state.height_angle_rad.to_degrees(),
            near = self.state.near_plane,
            far = self.state.far_plane,
            "sync_with_settings applied"
        );

        self.lmb_was_down_scene = false;
        self.lmb_dragging_scene = false;
        self.rmb_dragging_scene = false;
        self.last_cursor_vp_for_drag = None;
        self.pending_wheel_lines = 0.0;
    }

    pub fn snap_to_view(&mut self, view: CameraSnapView, settings: &CameraSettings) {
        self.cancel_animation();
        let qc = view.orientation();
        let q_end = canonical_quat_to_world(&self.axes, qc);
        let focal = self.state.focal_point_dvec(&self.axes);
        let fd = self.state.focal_distance;
        let fwd_end = rotate_vec_by_quat(self.axes.depth().vector() * -1.0, q_end.normalize());
        let eye_end =
            focal - DVec3::new(fwd_end.x as f64, fwd_end.y as f64, fwd_end.z as f64) * fd;

        let e0 = self.state.eye;
        let q0 = self.state.orientation;
        self.tween = CameraTween::begin(e0, eye_end, q0, q_end, fd, fd, settings);
    }

    pub fn orient_to_plane(
        &mut self,
        plane_origin: Vec3,
        plane_normal: Vec3,
        plane_up: Vec3,
        settings: &CameraSettings,
    ) {
        let normal = plane_normal.normalize();
        let up = plane_up.normalize();
        let forward = -normal;
        let right = up.cross(forward).normalize_or_zero();
        if right.length_squared() < 1e-12 {
            return;
        }
        let cam_up = forward.cross(right).normalize();
        let rot = Mat3::from_cols(right, cam_up, forward);
        let q_end = Quat::from_mat3(&rot).normalize();

        let fd = self.state.focal_distance;
        let focal = DVec3::new(
            plane_origin.x as f64,
            plane_origin.y as f64,
            plane_origin.z as f64,
        );
        let fwd_end =
            rotate_vec_by_quat(self.axes.depth().vector() * -1.0, q_end);
        let eye_end =
            focal - DVec3::new(fwd_end.x as f64, fwd_end.y as f64, fwd_end.z as f64) * fd;

        self.cancel_animation();
        self.tween = CameraTween::begin(
            self.state.eye,
            eye_end,
            self.state.orientation,
            q_end,
            fd,
            fd,
            settings,
        );
    }

    pub fn apply_rotate_delta(&mut self, delta: &RotateDelta, settings: &CameraSettings) {
        let angle = delta.degrees.to_radians();
        let current = self.state.orientation;
        let axis_world = match delta.axis {
            RotateAxis::ScreenX => current * (-math::control_horizontal_vec(&self.axes)),
            RotateAxis::ScreenY => current * (-self.axes.vertical().vector()),
            RotateAxis::ScreenZ => current * self.axes.depth().vector(),
        };
        if axis_world.length_squared() <= 0.0 {
            return;
        }
        let qdelta = Quat::from_axis_angle(axis_world.normalize(), angle);
        let q_end = (qdelta * current).normalize();
        let e0 = self.state.eye;
        let focal = self.state.focal_point_dvec(&self.axes);
        let fd = self.state.focal_distance;

        let fwd_end = rotate_vec_by_quat(self.axes.depth().vector() * -1.0, q_end);
        let eye_end =
            focal - DVec3::new(fwd_end.x as f64, fwd_end.y as f64, fwd_end.z as f64) * fd;

        self.cancel_animation();
        self.tween = CameraTween::begin(e0, eye_end, current, q_end, fd, fd, settings);
    }

    pub fn position(&self) -> [f32; 3] {
        self.position_vec().to_array()
    }

    /// Focal pivot (/workbench "target").
    pub fn target(&self) -> [f32; 3] {
        self.focal_point_world().to_array()
    }

    pub fn orientation(&self) -> [f32; 4] {
        self.state.orientation.normalize().to_array()
    }

    pub fn axis_system(&self) -> AxisSystem {
        self.axes
    }

    pub(crate) fn position_vec(&self) -> Vec3 {
        self.state.eye_vec3()
    }
}

fn rotate_vec_by_quat(v: Vec3, q: Quat) -> Vec3 {
    q * v
}

#[derive(Debug)]
pub enum CameraPointerResult {
    None,
    Redraw,
    /// Caller should fire select-on-click heuristics (only if no drag happened).
    LmbReleasedMaybeSelect,
}

impl CameraPointerResult {
    pub(crate) fn wants_redraw(&self) -> bool {
        matches!(
            self,
            CameraPointerResult::Redraw | CameraPointerResult::LmbReleasedMaybeSelect
        )
    }
}

#[cfg(test)]
mod navigation_tests;
