//! Timed view changes through the shared camera's transitions, relative to
//! a model origin frozen at their start. The host owns the clock.

use std::time::Duration;

use glam::{DVec3, Quat};

use super::hold;
use super::{AxisFrame, ViewSettings, ViewState};
use crate::transition::CameraTransition;

#[derive(Clone, Debug, Default)]
pub struct ViewTween {
    running: Option<(DVec3, CameraTransition)>,
}

impl ViewTween {
    /// A transition to `end_eye` and `end_q` at `end_fd`, keeping the lens.
    pub fn begin(
        state: &ViewState,
        frame: &AxisFrame,
        end_eye: DVec3,
        end_q: Quat,
        end_fd: f64,
        settings: &ViewSettings,
    ) -> Self {
        let mut target = state.clone();
        target.eye = end_eye;
        target.orientation = end_q;
        target.focal_distance = end_fd;
        Self::toward(state, frame, &target, settings)
    }

    /// A transition from `state` to the whole of `target`: its lens and
    /// zoom as much as its pose.
    pub fn toward(
        state: &ViewState,
        frame: &AxisFrame,
        target: &ViewState,
        settings: &ViewSettings,
    ) -> Self {
        let origin = state.eye;
        let source = hold::camera(state, frame, origin);
        let target = hold::camera(target, frame, origin);
        let duration = Duration::from_secs_f32((settings.view_transition_ms / 1000.0).max(0.0));
        Self {
            running: Some((origin, CameraTransition::frame(source, target, duration))),
        }
    }

    pub fn is_active(&self) -> bool {
        self.running.is_some()
    }

    pub fn cancel(&mut self) {
        self.running = None;
    }

    /// Put the view where the transition ends and stop it.
    pub fn finish(&mut self, state: &mut ViewState, frame: &AxisFrame) {
        if let Some((origin, mut transition)) = self.running.take() {
            transition.advance(Duration::MAX);
            hold::apply(state, frame, origin, transition.sample());
        }
    }

    /// Advance by `dt_secs` and put the view there; whether one was running.
    pub fn tick(&mut self, dt_secs: f32, state: &mut ViewState, frame: &AxisFrame) -> bool {
        let Some((origin, transition)) = &mut self.running else {
            return false;
        };
        transition.advance(Duration::from_secs_f32(dt_secs.max(0.0)));
        hold::apply(state, frame, *origin, transition.sample());
        if transition.is_done() {
            self.running = None;
        }
        true
    }
}
