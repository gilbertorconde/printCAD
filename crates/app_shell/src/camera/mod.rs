//! CAD-style viewport camera for printCAD (`docs/CAMERA.md`).
//!
//! Invariants worth preserving:
//! - **`focal_distance`** measured along **`forward`** (eye→focal ray), not spherical radius quirks.
//! - **Pan** translates `eye`; focal point stays `eye + forward * focal_distance` (orientation fixed).
//! - **Perspective zoom** dolls along **forward** so the pivot stays fixed unless zoom-to-cursor corrects.
//!
//! `view_projection` bakes Vulkan's Y-down clip space into the matrix; the
//! helpers that project and unproject through it live in
//! `core_document::runtime`, and every caller goes through them.

mod animate;
mod auto_clip;
mod core;
mod math;
mod ops;
pub(crate) mod section;
pub(crate) mod state;
mod zoom_cursor;

use crate::orientation_cube::{CameraSnapView, RotateAxis, RotateDelta};
use animate::CameraTween;
use axes::{AxisPreset, AxisSystem};
use glam::{DVec3, Mat3, Quat, Vec2, Vec3};
use settings::{CameraSettings, ProjectionMode, SixDofMotion, SixDofSettings};
use state::{CadCameraState, canonical_quat_to_world};
use tracing::{debug, trace};
use viewport_camera::controller::GestureKind;
use winit::event::{MouseButton, MouseScrollDelta, WindowEvent};

/// Normalizes one device reading: full scale to ±1, dead zone to rest,
/// inverted axes flipped, and the dominant axis alone when asked for.
/// `None` when the puck is at rest.
fn device_axes(readings: [f32; 6], device: &SixDofSettings) -> Option<[f32; 6]> {
    let scale = device.full_scale.abs().max(1.0);
    let dead_zone = device.dead_zone.clamp(0.0, 0.95);

    let mut axes = [0.0f32; 6];
    for (index, raw) in readings.iter().enumerate() {
        let mut value = raw / scale;
        if device.invert[index] {
            value = -value;
        }
        if value.abs() < dead_zone {
            value = 0.0;
        }
        axes[index] = value.clamp(-1.0, 1.0);
    }

    if device.dominant_axis {
        let mut strongest = 0;
        for index in 1..axes.len() {
            if axes[index].abs() > axes[strongest].abs() {
                strongest = index;
            }
        }
        let kept = axes[strongest];
        axes = [0.0; 6];
        axes[strongest] = kept;
    }

    axes.iter().any(|value| *value != 0.0).then_some(axes)
}

/// The field of view a perspective takes, in degrees: from nearly flat to wide.
pub const FOV_RANGE_DEG: (f32, f32) = (10.0, 120.0);

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
    /// True once movement from [`lmb_anchor_vp`] crosses `click_drag_threshold_px`.
    lmb_dragging_scene: bool,
    /// Where the right button went down, so its release can tell a click
    /// from a pan.
    rmb_anchor_vp: Vec2,
    rmb_dragging_scene: bool,
    lmb_dragging_roll: bool,
    lmb_was_down_scene: bool,
    /// Orbit pivot (world) picked under the cursor when a middle-button orbit
    /// starts with [`settings::CameraSettings::orbit_pivot_pick`] on; rotates
    /// the eye about this point without reframing.
    orbit_anchor_world: Option<DVec3>,
    /// Out-of-plane rotation disabled (sketch editing keeps the view planar).
    orbit_locked: bool,
    /// MMB drag state (orbit lives on the middle button; a clean MMB click
    /// picks the orbit pivot instead).
    mmb_was_down_scene: bool,
    mmb_anchor_vp: Vec2,
    mmb_dragging_scene: bool,
    navigation_hold: Option<core::Hold>,
    pixels_per_point: f32,
    roll_hold: Option<(Vec2, Quat)>,
}

impl CameraController {
    pub fn new(settings: &CameraSettings, initial_viewport: (u32, u32)) -> Self {
        Self {
            state: CadCameraState::new(settings, initial_viewport),
            axes: AxisSystem::from(settings.axis_preset),
            axis_preset: settings.axis_preset,
            tween: CameraTween::default(),
            scene_aabb: None,
            pending_wheel_lines: 0.0,
            last_cursor_viewport: None,
            last_cursor_vp_for_drag: None,
            lmb_anchor_vp: Vec2::ZERO,
            lmb_dragging_scene: false,
            rmb_anchor_vp: Vec2::ZERO,
            rmb_dragging_scene: false,
            lmb_dragging_roll: false,
            lmb_was_down_scene: false,
            orbit_anchor_world: None,
            orbit_locked: false,
            mmb_was_down_scene: false,
            mmb_anchor_vp: Vec2::ZERO,
            mmb_dragging_scene: false,
            navigation_hold: None,
            pixels_per_point: 1.0,
            roll_hold: None,
        }
    }

    /// Whether a view transition is animating; such frames must keep coming.
    pub fn is_animating(&self) -> bool {
        self.tween.is_active()
    }

    pub(crate) fn cancel_animation(&mut self) {
        self.tween.cancel();
    }

    /// Lock out-of-plane rotation (the middle-button orbit). Pan, zoom, and roll stay
    /// available: they keep the view planar. Used while editing a sketch.
    pub fn set_orbit_lock(&mut self, locked: bool) {
        if locked != self.orbit_locked {
            self.navigation_hold = None;
            self.mmb_was_down_scene = false;
            self.mmb_dragging_scene = false;
        }
        self.orbit_locked = locked;
    }

    pub fn set_pixels_per_point(&mut self, scale: f32) {
        assert!(scale.is_finite() && scale > 0.0);
        if scale != self.pixels_per_point {
            self.forget_released([false; 3]);
            self.pixels_per_point = scale;
        }
    }

    /// Let go of every drag whose button is no longer down (`[left, right,
    /// middle]`). A release something else took (a panel, a dialog, the
    /// bench) never reaches [`Self::on_viewport_pointer`], and the button
    /// would otherwise stay held here: a stale left turns the next right
    /// drag into a roll, a stale middle orbits on any move. Called before a
    /// move or a press, never a release, which needs its button still held.
    pub fn forget_released(&mut self, [left, right, middle]: [bool; 3]) {
        if !left {
            self.roll_hold = None;
            self.lmb_was_down_scene = false;
            self.lmb_dragging_scene = false;
            self.lmb_dragging_roll = false;
        }
        if !right {
            self.roll_hold = None;
            self.rmb_dragging_scene = false;
            if !middle {
                self.navigation_hold = None;
            }
        }
        if !middle {
            self.mmb_was_down_scene = false;
            self.mmb_dragging_scene = false;
        }
        if !(left || right || middle) {
            self.navigation_hold = None;
            self.last_cursor_vp_for_drag = None;
            self.orbit_anchor_world = None;
        }
    }

    /// Begin / update pointer drag modes: MMB orbit (a clean MMB click picks the pivot), LMB select, RMB pan, LMB+RMB tilt (roll).
    ///
    /// A middle-button orbit starts once the pointer moves
    /// `CameraSettings::click_drag_threshold_px` from where the button went down (over geometry or
    /// empty space); a shorter middle click picks the pivot, and a left press that moves less than
    /// that is a selection click.
    ///
    /// When `CameraSettings::orbit_pivot_pick` is set, the first orbit-drag frame remembers
    /// `pick_world_under_cursor` as an off-axis orbit pivot (no recenter jump).
    pub fn on_viewport_pointer(
        &mut self,
        event: &WindowEvent,
        settings: &CameraSettings,
        pick_world_under_cursor: Option<Vec3>,
    ) -> CameraPointerResult {
        match event {
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: winit::event::ElementState::Pressed,
                ..
            } => {
                self.cancel_animation();
                self.lmb_was_down_scene = true;
                self.navigation_hold = None;
                self.orbit_anchor_world = None;
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
                if let Some(p) = self.last_cursor_viewport {
                    self.rmb_anchor_vp = p;
                    self.navigation_hold = Some(core::Hold::begin(
                        &self.state,
                        &self.axes,
                        settings,
                        p,
                        self.state.focal_point_dvec(&self.axes),
                        GestureKind::Pan,
                        self.pixels_per_point,
                    ));
                }
                CameraPointerResult::Redraw
            }
            WindowEvent::MouseInput {
                button: MouseButton::Middle,
                state: winit::event::ElementState::Pressed,
                ..
            } => {
                self.cancel_animation();
                self.mmb_was_down_scene = true;
                self.mmb_dragging_scene = false;
                self.orbit_anchor_world = None;
                if let Some(p) = self.last_cursor_viewport {
                    self.mmb_anchor_vp = p;
                    self.last_cursor_vp_for_drag = Some(p);
                    let focal = self.state.focal_point_dvec(&self.axes);
                    let anchor = if settings.orbit_pivot_pick {
                        pick_world_under_cursor
                            .filter(|hit| {
                                (*hit - self.state.eye_vec3())
                                    .dot(self.state.forward_world(&self.axes))
                                    > 0.0
                            })
                            .map(|hit| hit.as_dvec3())
                            .unwrap_or(focal)
                    } else {
                        focal
                    };
                    self.orbit_anchor_world = Some(anchor);
                    self.navigation_hold = Some(core::Hold::begin(
                        &self.state,
                        &self.axes,
                        settings,
                        p,
                        anchor,
                        GestureKind::Orbit,
                        self.pixels_per_point,
                    ));
                }
                CameraPointerResult::Redraw
            }
            WindowEvent::MouseInput {
                button: MouseButton::Middle,
                state: winit::event::ElementState::Released,
                ..
            } => {
                let was_click = self.mmb_was_down_scene && !self.mmb_dragging_scene;
                self.mmb_was_down_scene = false;
                self.mmb_dragging_scene = false;
                self.navigation_hold = None;
                self.last_cursor_vp_for_drag = None;
                self.orbit_anchor_world = None;
                if was_click && self.last_cursor_viewport.is_some() {
                    // A clean middle click (no drag) picks the orbit pivot.
                    self.on_mmb_pivot_pick(pick_world_under_cursor, settings);
                    return CameraPointerResult::Redraw;
                }
                CameraPointerResult::None
            }
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: winit::event::ElementState::Released,
                ..
            } => {
                let should_maybe_select = !self.lmb_dragging_scene && !self.lmb_dragging_roll;
                let was_dragging = self.lmb_dragging_scene;

                self.last_cursor_vp_for_drag = None;
                self.lmb_dragging_roll = false;
                self.lmb_was_down_scene = false;
                self.navigation_hold = None;
                self.lmb_dragging_scene = false;
                self.orbit_anchor_world = None;

                if should_maybe_select && self.last_cursor_viewport.is_some() {
                    return CameraPointerResult::LmbReleasedMaybeSelect;
                }
                if was_dragging {
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
                let was_down = self.rmb_dragging_scene;
                self.last_cursor_vp_for_drag = None;
                self.rmb_dragging_scene = false;
                self.lmb_dragging_roll = false;
                self.navigation_hold = None;
                // A press and release in the same place is a click, not a
                // pan, whatever few pixels the pan moved on the way.
                let threshold = settings.click_drag_threshold_px * self.pixels_per_point;
                let clicked = was_down
                    && !self.lmb_was_down_scene
                    && self.last_cursor_viewport.is_some_and(|p| {
                        (p - self.rmb_anchor_vp).length_squared() <= threshold * threshold
                    });
                if clicked {
                    return CameraPointerResult::RmbReleasedMaybeMenu;
                }
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
                    self.navigation_hold = None;
                    self.lmb_dragging_roll = true;
                    let (start, orientation) =
                        *self.roll_hold.get_or_insert((last, self.state.orientation));
                    self.state.orientation = orientation;
                    ops::roll_pixels(
                        &mut self.state,
                        &self.axes,
                        (cur.x - start.x) / self.pixels_per_point,
                    );
                    self.last_cursor_vp_for_drag = Some(cur);
                    return CameraPointerResult::Redraw;
                }

                if self.rmb_dragging_scene && !self.lmb_was_down_scene {
                    let hold = self.navigation_hold.get_or_insert_with(|| {
                        core::Hold::begin(
                            &self.state,
                            &self.axes,
                            settings,
                            last,
                            self.state.focal_point_dvec(&self.axes),
                            GestureKind::Pan,
                            self.pixels_per_point,
                        )
                    });
                    hold.sample(&mut self.state, &self.axes, cur);
                    self.last_cursor_vp_for_drag = Some(cur);
                    self.state.clip_dirty = true;
                    return CameraPointerResult::Redraw;
                }

                if self.mmb_was_down_scene {
                    let thresh_sq =
                        (settings.click_drag_threshold_px * self.pixels_per_point).powi(2);
                    let exceeds_anchor = (cur - self.mmb_anchor_vp).length_squared() >= thresh_sq;
                    if exceeds_anchor && !self.mmb_dragging_scene && !self.orbit_locked {
                        self.mmb_dragging_scene = true;
                    }
                    if self.mmb_dragging_scene {
                        let hold = self
                            .navigation_hold
                            .expect("middle press acquires its navigation snapshot");
                        hold.sample(&mut self.state, &self.axes, cur);
                        self.last_cursor_vp_for_drag = Some(cur);
                        self.state.clip_dirty = true;
                        return CameraPointerResult::Redraw;
                    }
                }

                // LMB drag does not orbit (the button belongs to selection)
                // but still tracks the click-vs-drag threshold so a drag
                // doesn't count as a selection click on release.
                if self.lmb_was_down_scene && !self.lmb_dragging_scene {
                    let thresh_sq =
                        (settings.click_drag_threshold_px * self.pixels_per_point).powi(2);
                    if (cur - self.lmb_anchor_vp).length_squared() >= thresh_sq {
                        self.lmb_dragging_scene = true;
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

    /// The cursor's position, in viewport-local physical pixels.
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
        if let Some(hold) = self.navigation_hold
            && let Some(pointer) = self.last_cursor_viewport
        {
            let hold = hold.zoomed(lines, settings);
            hold.sample(&mut self.state, &self.axes, pointer);
            self.navigation_hold = Some(hold);
            return;
        }
        zoom_cursor::apply_zoom_wheels(
            &mut self.state,
            &self.axes,
            self.last_cursor_viewport,
            lines,
            settings,
        );
    }

    /// Steer the view with one reading from a 6-DoF mouse.
    ///
    /// `axis_readings` is the device's own six numbers, three translations
    /// then three rotations, as the daemon sends them. They describe a rate,
    /// not a step: the puck is held at a deflection and the view moves for as
    /// long as it is held, so each reading is integrated over the frame.
    /// Returns whether the view moved.
    pub fn apply_device_motion(
        &mut self,
        axis_readings: [f32; 6],
        dt_secs: f32,
        settings: &CameraSettings,
        device: &SixDofSettings,
    ) -> bool {
        if !device.enabled {
            return false;
        }
        let Some(axes) = device_axes(axis_readings, device) else {
            return false;
        };

        // Each of the puck's six movements adds to whatever it is assigned
        // to, so two of them can drive the same motion if that is what the
        // assignment says.
        let mut pan = Vec2::ZERO;
        let mut zoom = 0.0f32;
        let mut orbit = Vec2::ZERO;
        let mut roll = 0.0f32;
        for ((reading, motion), speed) in axes.iter().zip(&device.assign).zip(&device.speed) {
            let amount = reading * speed;
            match motion {
                SixDofMotion::None => {}
                SixDofMotion::PanSideways => pan.x += amount,
                SixDofMotion::PanUpDown => pan.y -= amount,
                SixDofMotion::Zoom => zoom += amount,
                SixDofMotion::Tilt => orbit.y -= amount,
                SixDofMotion::Turn => orbit.x -= amount,
                SixDofMotion::Roll => roll += amount,
            }
        }

        // A frame after waking from sleep can be arbitrarily long; integrating
        // it whole would throw the view across the scene.
        let dt = dt_secs.clamp(0.001, 0.1);
        self.cancel_animation();

        // Pan is already in pixels per second.
        if pan != Vec2::ZERO {
            ops::pan_pixels(&mut self.state, &self.axes, pan * dt, settings);
        }

        // Zoom is in wheel steps per second; positive zooms in, the way
        // scrolling up does.
        if zoom != 0.0 {
            zoom_cursor::apply_zoom_wheels(&mut self.state, &self.axes, None, zoom * dt, settings);
        }

        // The turns arrive in degrees per second, and orbit takes pixels:
        // `orbit_sensitivity * 0.005` is the radians one pixel turns, so go
        // the other way.
        let radians_per_px = (settings.orbit_sensitivity * 0.005).max(1e-6);

        // A sketch locks the view to its plane: the two movements that would
        // tilt out of it are dropped, and the one about the plane's own
        // normal, roll, is kept.
        if orbit != Vec2::ZERO && !self.orbit_locked {
            let delta = orbit * dt * std::f32::consts::PI / 180.0 / radians_per_px;
            match self.orbit_anchor_world {
                Some(pivot) if settings.orbit_pivot_pick => {
                    ops::orbit_pixels_around_world_anchor(
                        &mut self.state,
                        &self.axes,
                        pivot,
                        delta,
                        settings,
                    );
                }
                _ => ops::orbit_pixels(&mut self.state, &self.axes, delta, settings),
            }
        }

        if roll != 0.0 {
            // Roll works in its own units, where one pixel is 0.01 radians.
            ops::roll_pixels(&mut self.state, &self.axes, (roll * dt).to_radians() / 0.01);
        }

        true
    }

    /// A middle click's pivot pick, at the world point under the cursor the
    /// caller supplies.
    pub fn on_mmb_pivot_pick(&mut self, world_hit: Option<Vec3>, settings: &CameraSettings) {
        if let Some(hit) = world_hit
            && ops::set_pivot_world_hit(&mut self.state, &self.axes, hit, settings)
        {
            self.state.clip_dirty = true;
            self.cancel_animation();
        }
    }

    pub fn pivot_from_key_h(&mut self, settings: &CameraSettings) -> bool {
        let Some(vp) = self.last_cursor_viewport else {
            return false;
        };
        self.cancel_animation();
        ops::set_pivot_focal_plane_cursor(&mut self.state, &self.axes, vp, settings)
    }

    pub fn update(&mut self, dt_secs: f32, _settings: &CameraSettings) -> bool {
        let had = self.tween.tick(dt_secs, &mut self.state, &self.axes);
        if had {
            self.state.clip_dirty = true;
            true
        } else {
            false
        }
    }

    pub fn update_viewport(&mut self, origin: (u32, u32), size: (u32, u32)) {
        if self.state.viewport_size != size {
            self.forget_released([false; 3]);
        }
        self.state.viewport_origin = (origin.0 as f32, origin.1 as f32);
        self.state.viewport_size = size;
        self.state.clip_dirty = true;
    }

    pub fn view_projection(&self) -> [[f32; 4]; 4] {
        self.state.view_projection(&self.axes).to_cols_array_2d()
    }

    /// Viewport-local pixel position of a world point: the space of
    /// `cursor_in_viewport` and of the screen-space overlays, which the
    /// painters offset by the viewport origin themselves. `None` when the
    /// point is behind the camera or the viewport has no size.
    pub fn world_to_viewport(&self, world_pos: Vec3) -> Option<(f32, f32)> {
        let vp = self.viewport_info();
        core_document::runtime::world_to_viewport(
            self.view_projection(),
            (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            world_pos.to_array(),
        )
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

    /// Red crosshair at orbit focal point: only while orbiting (MMB), rolling (LMB+RMB) or animating a view.
    pub fn rotation_pivot_marker_visible(&self) -> bool {
        self.mmb_dragging_scene || self.lmb_dragging_roll || self.tween.is_active()
    }

    /// Physical pixel position for the red orbit pivot HUD (matches [`Self::world_to_screen`] space).
    ///
    /// When [`CameraSettings::orbit_pivot_pick`] is on and the orbit rotates about a picked surface
    /// point, the indicator is drawn at that **anchor** projected to screen (not under the cursor
    /// while dragging).
    pub fn rotation_pivot_indicator_screen_px(&self, orbit_pivot_pick: bool) -> Option<(f32, f32)> {
        if !self.rotation_pivot_marker_visible() {
            return None;
        }
        if orbit_pivot_pick
            && self.mmb_dragging_scene
            && let Some(p) = self.orbit_anchor_world
        {
            let world = Vec3::new(p.x as f32, p.y as f32, p.z as f32);
            if let Some(px) = self.world_to_screen(world) {
                return Some(px);
            }
        }
        self.world_to_screen(Vec3::from_array(self.target()))
    }

    pub fn reset_to_fit(
        &mut self,
        center: Vec3,
        radius: f32,
        zoom_limit_aabb: Option<(Vec3, Vec3)>,
        settings: &CameraSettings,
    ) {
        self.forget_released([false; 3]);
        self.cancel_animation();
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

    /// Frame the sphere without turning the view or leaving the focal
    /// plane: the centre slides along the view onto it. What an edit
    /// session that holds the view square to its plane fits with, the
    /// plane being where the session put the focal point.
    pub fn fit_keeping_orientation(
        &mut self,
        center: Vec3,
        radius: f32,
        settings: &CameraSettings,
    ) {
        self.forget_released([false; 3]);
        // A turn under way (onto the plane, as a session opens) is the
        // orientation to keep.
        self.tween.finish(&mut self.state, &self.axes);
        let (forward, _) = self.view_basis();
        let center = center - forward * (center - self.focal_point_world()).dot(forward);
        ops::fit_sphere_as_oriented(&mut self.state, &self.axes, center, radius, settings);
        self.after_scene_or_settings_touch(settings);
    }

    /// The box around what the scene draws, which the clip planes enclose.
    /// The host sets it every frame; a change re-derives the planes.
    pub fn set_scene_bounds(&mut self, bounds: Option<(Vec3, Vec3)>) {
        if self.scene_aabb != bounds {
            self.scene_aabb = bounds;
            self.state.clip_dirty = true;
        }
    }

    /// The box last given by [`Self::set_scene_bounds`].
    pub fn scene_bounds(&self) -> Option<(Vec3, Vec3)> {
        self.scene_aabb
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

    fn after_scene_or_settings_touch(&mut self, _settings: &CameraSettings) {
        self.state.clip_dirty = true;
    }

    /// The perspective's vertical field of view, in degrees.
    pub fn field_of_view_deg(&self) -> f32 {
        self.state.height_angle_rad.to_degrees() as f32
    }

    /// Change the perspective's field of view and keep the framing: the eye
    /// moves along the view so what stands at the focal point keeps its size
    /// on screen, and only the perspective's strength changes. Nothing
    /// happens in an orthographic view, which has no field of view.
    pub fn set_field_of_view(&mut self, degrees: f32, settings: &CameraSettings) {
        if self.state.projection != ProjectionMode::Perspective {
            return;
        }
        self.forget_released([false; 3]);
        let new = f64::from(degrees.clamp(FOV_RANGE_DEG.0, FOV_RANGE_DEG.1)).to_radians();
        let old = self.state.height_angle_rad;
        let focal = self.state.focal_point_dvec(&self.axes);
        self.cancel_animation();
        self.state.focal_distance *= (old * 0.5).tan() / (new * 0.5).tan();
        self.state.clamp_focal_distance(settings);
        self.state.height_angle_rad = new;
        self.state.rederive_eye_from_focal(focal, &self.axes);
        self.state.clip_dirty = true;
    }

    pub fn sync_with_settings(&mut self, settings: &CameraSettings) {
        if self.axis_preset != settings.axis_preset {
            self.axis_preset = settings.axis_preset;
            self.axes = AxisSystem::from(self.axis_preset);
        }
        self.state.height_angle_rad = (settings.fov_degrees as f64).to_radians();
        self.state.ortho_height = settings.ortho_height_mm as f64;

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
        self.forget_released([false; 3]);
    }

    pub fn snap_to_view(&mut self, view: CameraSnapView, settings: &CameraSettings) {
        self.forget_released([false; 3]);
        self.cancel_animation();
        let qc = view.orientation();
        let q_end = canonical_quat_to_world(&self.axes, qc);
        let focal = self.state.focal_point_dvec(&self.axes);
        let fd = self.state.focal_distance;
        let fwd_end = rotate_vec_by_quat(self.axes.depth().vector() * -1.0, q_end.normalize());
        let eye_end = focal - DVec3::new(fwd_end.x as f64, fwd_end.y as f64, fwd_end.z as f64) * fd;

        self.tween = CameraTween::begin(&self.state, &self.axes, eye_end, q_end, fd, settings);
    }

    pub fn orient_to_plane(
        &mut self,
        plane_origin: Vec3,
        plane_normal: Vec3,
        plane_up: Vec3,
        centre: Option<Vec3>,
        settings: &CameraSettings,
    ) {
        self.forget_released([false; 3]);
        let normal = plane_normal.normalize();
        // Look at the plane from its +normal side, with the plane's up axis
        // (orthogonalized against the view direction) as screen-up.
        let forward = -normal;
        let up_raw = plane_up - forward * plane_up.dot(forward);
        if up_raw.length_squared() < 1e-12 {
            return;
        }
        let cam_up = up_raw.normalize();

        // The camera state consumes the orientation as `q * (-depth) =
        // forward` and `q * vertical = up` in the ACTIVE axis preset, not
        // the canonical XYZ basis, which rolls the view under the default
        // Z-up preset. Build q as the rotation mapping the preset pair onto
        // the target pair.
        let depth = self.axes.depth().vector();
        let vertical = self.axes.vertical().vector();
        let src_forward = -depth;
        let src = Mat3::from_cols(src_forward, vertical, src_forward.cross(vertical));
        let dst = Mat3::from_cols(forward, cam_up, forward.cross(cam_up));
        let q_end = Quat::from_mat3(&(dst * src.transpose())).normalize();

        let fd = self.state.focal_distance;
        // The point asked for, else what the view was on, carried square
        // onto the plane. A sketch plane's origin is where the world origin
        // falls on it, which for a face away from the origin is nowhere
        // near the face.
        let centre = centre.unwrap_or_else(|| {
            let target = self.focal_point_world();
            target - normal * (target - plane_origin).dot(normal)
        });
        let focal = DVec3::new(centre.x as f64, centre.y as f64, centre.z as f64);
        let fwd_end = rotate_vec_by_quat(self.axes.depth().vector() * -1.0, q_end);
        let eye_end = focal - DVec3::new(fwd_end.x as f64, fwd_end.y as f64, fwd_end.z as f64) * fd;

        self.cancel_animation();
        self.tween = CameraTween::begin(&self.state, &self.axes, eye_end, q_end, fd, settings);
    }

    pub fn apply_rotate_delta(&mut self, delta: &RotateDelta, settings: &CameraSettings) {
        self.forget_released([false; 3]);
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
        let focal = self.state.focal_point_dvec(&self.axes);
        let fd = self.state.focal_distance;

        let fwd_end = rotate_vec_by_quat(self.axes.depth().vector() * -1.0, q_end);
        let eye_end = focal - DVec3::new(fwd_end.x as f64, fwd_end.y as f64, fwd_end.z as f64) * fd;

        self.cancel_animation();
        self.tween = CameraTween::begin(&self.state, &self.axes, eye_end, q_end, fd, settings);
    }

    pub fn position(&self) -> [f32; 3] {
        self.position_vec().to_array()
    }

    /// The focal pivot: the point the view looks at and orbits about,
    /// handed to benches as the camera's target.
    pub fn target(&self) -> [f32; 3] {
        self.focal_point_world().to_array()
    }

    pub fn orientation(&self) -> [f32; 4] {
        self.state.orientation.normalize().to_array()
    }

    /// The view's forward and up directions in world space, as the
    /// orientation maps the axis preset's depth and vertical.
    pub fn view_basis(&self) -> (Vec3, Vec3) {
        let q = self.state.orientation.normalize();
        (
            q * -self.axes.depth().vector(),
            q * self.axes.vertical().vector(),
        )
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
    /// The right button came up where it went down: a click, not a pan.
    RmbReleasedMaybeMenu,
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

#[cfg(test)]
mod integration_tests;
