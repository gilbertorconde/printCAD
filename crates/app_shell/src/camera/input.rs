use glam::Vec2;
use settings::{CameraSettings, MouseButtonSetting};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};

use super::controller::CameraController;
impl CameraController {
    pub fn handle_event(&mut self, event: &WindowEvent, settings: &CameraSettings) -> bool {
        match event {
            WindowEvent::MouseInput { state, button, .. } => {
                let orbit_button = mouse_button_from_setting(settings.orbit_button);
                let pan_button = mouse_button_from_setting(settings.pan_button);
                let pressed = matches!(state, ElementState::Pressed);
                match (button, pressed) {
                    (b, true) if *b == orbit_button => {
                        self.orbiting = true;
                        self.animation = None; // user input overrides animation
                                               // Capture the current pivot point for this orbit session
                        self.active_pivot = self.orbit_pivot;
                        true
                    }
                    (b, false) if *b == orbit_button => {
                        self.orbiting = false;
                        self.last_cursor = None;
                        self.active_pivot = None;
                        true
                    }
                    (b, true) if *b == pan_button => {
                        self.panning = true;
                        true
                    }
                    (b, false) if *b == pan_button => {
                        self.panning = false;
                        self.last_cursor = None;
                        true
                    }
                    _ => false,
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let changed = self.handle_cursor_moved(*position, settings);
                if self.orbiting || self.panning {
                    self.last_cursor = Some(*position);
                } else {
                    self.last_cursor = None;
                }
                changed
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.handle_scroll(delta, settings);
                true
            }
            WindowEvent::Resized(size) => {
                self.viewport_size = (size.width, size.height);
                false
            }
            _ => false,
        }
    }

    fn handle_cursor_moved(
        &mut self,
        position: winit::dpi::PhysicalPosition<f64>,
        settings: &CameraSettings,
    ) -> bool {
        let last = match self.last_cursor {
            Some(pos) => pos,
            None => return false,
        };

        let delta = Vec2::new((position.x - last.x) as f32, (position.y - last.y) as f32);

        if self.orbiting {
            self.orbit_trackball(delta, settings);
            true
        } else if self.panning {
            self.pan(delta);
            true
        } else {
            false
        }
    }

    fn handle_scroll(&mut self, delta: &MouseScrollDelta, settings: &CameraSettings) {
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, y) => *y,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / 120.0,
        };
        self.zoom(amount, settings);
    }
}

fn mouse_button_from_setting(setting: MouseButtonSetting) -> MouseButton {
    match setting {
        MouseButtonSetting::Left => MouseButton::Left,
        MouseButtonSetting::Middle => MouseButton::Middle,
        MouseButtonSetting::Right => MouseButton::Right,
    }
}
