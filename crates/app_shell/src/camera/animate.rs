//! Interpolated view transitions (`camera_system.md` §9).

use glam::{DVec3, Quat};
use settings::CameraSettings;

use crate::camera::math::quat_normalized_sign_fix;

#[derive(Clone, Default)]
pub(crate) enum CameraTween {
    #[default]
    None,
    Running {
        progress: f32,
        duration: f32,
        start_eye: DVec3,
        end_eye: DVec3,
        start_q: Quat,
        end_q: Quat,
        start_ln_fd: f64,
        end_ln_fd: f64,
    },
}

fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl CameraTween {
    pub(crate) fn begin(
        start_eye: DVec3,
        end_eye: DVec3,
        start_q: Quat,
        end_q: Quat,
        start_fd: f64,
        end_fd: f64,
        settings: &CameraSettings,
    ) -> Self {
        let eq = quat_normalized_sign_fix(start_q, end_q);
        CameraTween::Running {
            progress: 0.0,
            duration: (settings.view_transition_ms / 1000.0).max(1e-3),
            start_eye,
            end_eye,
            start_q,
            end_q: eq,
            start_ln_fd: start_fd.max(1e-9).ln(),
            end_ln_fd: end_fd.max(1e-9).ln(),
        }
    }

    pub(crate) fn cancel(&mut self) {
        *self = CameraTween::None;
    }

    /// Advance animation; assigns `out_pose` each frame until complete.
    /// Returns true when pose was produced (including final frame).
    pub(crate) fn tick(&mut self, dt_secs: f32, out_pose: &mut (DVec3, Quat, f64)) -> bool {
        match self {
            CameraTween::None => false,
            CameraTween::Running {
                progress,
                duration,
                start_eye,
                end_eye,
                start_q,
                end_q,
                start_ln_fd,
                end_ln_fd,
            } => {
                *progress += dt_secs / duration.max(1e-9);
                let t = ease_in_out((*progress).min(1.0));
                let eye = start_eye.lerp(*end_eye, t as f64);
                let q = start_q.slerp(*end_q, t).normalize();
                let ln =
                    *start_ln_fd + (*end_ln_fd - *start_ln_fd) * t as f64;
                let fd = ln.exp();

                *out_pose = (eye, q, fd);

                if *progress >= 1.0 {
                    *self = CameraTween::None;
                }
                true
            }
        }
    }
}
