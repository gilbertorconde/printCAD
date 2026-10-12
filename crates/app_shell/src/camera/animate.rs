//! The host clock and frozen model origin for shared camera transitions.

use std::time::Duration;

use axes::AxisSystem;
use glam::{DVec3, Quat};
use settings::CameraSettings;
use viewport_camera::transition::CameraTransition;

use super::core;
use super::state::CadCameraState;

#[derive(Clone, Default)]
pub(crate) struct CameraTween {
    running: Option<(DVec3, CameraTransition)>,
}

impl CameraTween {
    pub(crate) fn begin(
        state: &CadCameraState,
        axes: &AxisSystem,
        end_eye: DVec3,
        end_q: Quat,
        end_fd: f64,
        settings: &CameraSettings,
    ) -> Self {
        let mut target = state.clone();
        target.eye = end_eye;
        target.orientation = end_q;
        target.focal_distance = end_fd;
        Self::toward(state, axes, &target, settings)
    }

    /// A transition from `state` to the whole of `target`: its lens and
    /// zoom as much as its pose.
    pub(crate) fn toward(
        state: &CadCameraState,
        axes: &AxisSystem,
        target: &CadCameraState,
        settings: &CameraSettings,
    ) -> Self {
        let origin = state.eye;
        let source = core::camera(state, axes, origin);
        let target = core::camera(target, axes, origin);
        let duration = Duration::from_secs_f32((settings.view_transition_ms / 1000.0).max(0.0));
        Self {
            running: Some((origin, CameraTransition::frame(source, target, duration))),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        self.running.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.running = None;
    }

    /// Put the camera where the transition ends and stop it.
    pub(crate) fn finish(&mut self, state: &mut CadCameraState, axes: &AxisSystem) {
        if let Some((origin, mut transition)) = self.running.take() {
            transition.advance(Duration::MAX);
            core::apply(state, axes, origin, transition.sample());
        }
    }

    pub(crate) fn tick(
        &mut self,
        dt_secs: f32,
        state: &mut CadCameraState,
        axes: &AxisSystem,
    ) -> bool {
        let Some((origin, transition)) = &mut self.running else {
            return false;
        };
        transition.advance(Duration::from_secs_f32(dt_secs.max(0.0)));
        core::apply(state, axes, *origin, transition.sample());
        if transition.is_done() {
            self.running = None;
        }
        true
    }
}
