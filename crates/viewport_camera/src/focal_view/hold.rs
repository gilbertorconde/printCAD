//! Conversion between the view's frame-relative `f64` state and the shared
//! camera's canonical local frame. A hold freezes `origin` at its press.

use glam::{DVec3, Mat3, Quat, Vec2};

use super::{AxisFrame, ProjectionMode, ViewSettings, ViewState};
use crate::camera::{CameraPose, Projection};
use crate::controller::{CameraController, CameraControllerConfig, GestureKind};
use crate::navigation::{NavigationDrag, NavigationSettings, PanGrip, ScreenPoint};

fn basis(frame: &AxisFrame) -> Quat {
    Quat::from_mat3(&Mat3::from_cols(
        frame.control_horizontal(),
        frame.vertical,
        frame.depth,
    ))
}

/// The shared camera standing where `state` stands, positions relative to `origin`.
pub fn camera(state: &ViewState, frame: &AxisFrame, origin: DVec3) -> CameraController {
    let pose = CameraPose {
        position: (state.eye - origin).as_vec3(),
        orientation: (state.orientation * basis(frame)).normalize(),
    };
    let viewport = Vec2::new(
        state.viewport_size.0.max(1) as f32,
        state.viewport_size.1.max(1) as f32,
    );
    let projection = match state.projection {
        ProjectionMode::Perspective => Projection::Perspective {
            vertical_fov: state.height_angle_rad as f32,
        },
        ProjectionMode::Orthographic => {
            crate::controller::orthographic(viewport, state.world_per_pixel() as f32)
        }
    };
    CameraController::new(
        pose,
        projection,
        viewport,
        pose.position + pose.forward() * state.focal_distance as f32,
    )
}

/// Put `state` where the shared `camera` stands, relative to `origin`.
pub fn apply(state: &mut ViewState, frame: &AxisFrame, origin: DVec3, camera: CameraController) {
    state.eye = origin + camera.pose().position.as_dvec3();
    state.orientation = (camera.pose().orientation * basis(frame).conjugate()).normalize();
    state.focal_distance = f64::from(-camera.pose().world_to_view(camera.pivot()).z);
    match camera.projection() {
        Projection::Perspective { vertical_fov } => {
            state.projection = ProjectionMode::Perspective;
            state.height_angle_rad = f64::from(vertical_fov);
        }
        Projection::Orthographic { bottom, top, .. } => {
            state.projection = ProjectionMode::Orthographic;
            state.ortho_height = f64::from(top - bottom);
        }
    }
    state.clip_dirty = true;
}

/// [`camera`] with the frame's vertical as world-up and the host's focal floor.
pub fn configured(
    state: &ViewState,
    frame: &AxisFrame,
    settings: &ViewSettings,
    origin: DVec3,
) -> CameraController {
    camera(state, frame, origin).with_config(CameraControllerConfig {
        world_up: frame.vertical,
        min_focus_distance: settings.min_focal_distance,
        ..CameraControllerConfig::default()
    })
}

/// One pan or orbit from its press to its release, sampled with absolute
/// pointer positions in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Hold {
    origin: DVec3,
    drag: NavigationDrag,
    pointer_scale: f32,
    motion_scale: f32,
    focal_distance: f64,
    orientation: Quat,
    fov: f64,
    ortho_height: f64,
    kind: GestureKind,
}

impl Hold {
    pub fn begin(
        state: &ViewState,
        frame: &AxisFrame,
        settings: &ViewSettings,
        start: Vec2,
        anchor: DVec3,
        kind: GestureKind,
        pixels_per_point: f32,
    ) -> Self {
        let origin = state.eye;
        let mut camera = camera(state, frame, origin);
        camera.set_viewport(camera.viewport() / pixels_per_point);
        let navigation = NavigationSettings {
            keep_horizon: settings.keep_horizon,
            orbit_speed: 1.0,
            pan_grip: PanGrip::PivotDepth,
            ..NavigationSettings::default()
        };
        let range = match state.projection {
            ProjectionMode::Perspective => [
                (f64::from(settings.min_focal_distance) / state.focal_distance).ln() as f32,
                (f64::from(settings.max_focal_distance) / state.focal_distance).ln() as f32,
            ],
            ProjectionMode::Orthographic => [
                (1e-9 / state.ortho_height).ln() as f32,
                (1e12 / state.ortho_height).ln() as f32,
            ],
        };
        let drag = navigation
            .begin_drag_with_up(
                camera,
                ScreenPoint(start / pixels_per_point),
                (anchor - origin).as_vec3(),
                kind,
                frame.vertical,
            )
            .with_zoom_range([range[0].min(0.0), range[1].max(0.0)]);
        Self {
            origin,
            drag,
            pointer_scale: pixels_per_point,
            motion_scale: if kind == GestureKind::Pan {
                settings.pan_sensitivity
            } else {
                settings.orbit_sensitivity
            },
            focal_distance: state.focal_distance,
            orientation: state.orientation,
            fov: state.height_angle_rad,
            ortho_height: state.ortho_height,
            kind,
        }
    }

    pub fn sample(self, state: &mut ViewState, frame: &AxisFrame, pointer: Vec2) {
        let start = self.drag.start().0;
        let point = start + (pointer / self.pointer_scale - start) * self.motion_scale;
        apply(
            state,
            frame,
            self.origin,
            self.drag.sample(ScreenPoint(point)),
        );
        // A navigation hold does not change the host's lens or axial focal
        // distance. Preserve those f64 quantities without a round trip through f32.
        let zoom = f64::from(self.drag.zoom()).exp();
        state.focal_distance = if state.projection == ProjectionMode::Perspective {
            self.focal_distance * zoom
        } else {
            self.focal_distance
        };
        state.height_angle_rad = self.fov;
        state.ortho_height = self.ortho_height
            * if state.projection == ProjectionMode::Orthographic {
                zoom
            } else {
                1.0
            };
        if self.kind == GestureKind::Pan || point == start {
            state.orientation = self.orientation;
        }
    }

    /// The hold with `lines` wheel lines of zoom added to its total.
    pub fn zoomed(mut self, lines: f32, settings: &ViewSettings) -> Self {
        let sign = if settings.invert_zoom { -1.0 } else { 1.0 };
        self.drag = self
            .drag
            .zoomed(lines * sign * settings.wheel_zoom_factor.ln());
        self
    }
}
