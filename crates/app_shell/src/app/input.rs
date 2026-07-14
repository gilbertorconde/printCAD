//! Window-event handling and workbench input dispatch.

use core_document::{MouseButton as WbMouseButton, WorkbenchId, WorkbenchInputEvent};
use glam::{Vec2, Vec3};
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use render_vk::RenderBackend;

use crate::camera::CameraPointerResult;
use crate::log_panel as app_log;
use crate::PrintCadApp;

impl PrintCadApp {
    /// Body of the winit `window_event` handler.
    pub(crate) fn handle_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.gfx.as_ref().map(|g| g.window_id) != Some(window_id) {
            return;
        }

        // Track modifiers and pressed-button count regardless of who consumes
        // the event: the undo system uses "no button held" as its snapshot
        // boundary, and a release swallowed by egui must still decrement.
        match &event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::MouseInput { state, .. } => match state {
                ElementState::Pressed => {
                    self.mouse_buttons_down = self.mouse_buttons_down.saturating_add(1);
                }
                ElementState::Released => {
                    self.mouse_buttons_down = self.mouse_buttons_down.saturating_sub(1);
                }
            },
            _ => {}
        }

        // Update picking + viewport-local cursor *before* egui. Cursor events can be marked
        // consumed while dragging UI; we still need consistent coords for 3D hit testing and
        // zoom-to-focal-plane math.
        if let WindowEvent::CursorMoved { position, .. } = &event {
            // `CursorMoved` is already [`PhysicalPosition`]; match renderer + viewport_rect.
            let phys_x = position.x.max(0.0).round() as u32;
            let phys_y = position.y.max(0.0).round() as u32;

            let vp = self.camera.viewport_info();
            let cursor_x = phys_x as f32 - vp.0;
            let cursor_y = phys_y as f32 - vp.1;

            if cursor_x >= 0.0
                && cursor_y >= 0.0
                && cursor_x < vp.2 as f32
                && cursor_y < vp.3 as f32
            {
                self.cursor_in_viewport = Some((cursor_x, cursor_y));
                // Only pick while the cursor is over the 3D viewport; over
                // egui panels the hover state is irrelevant and each pick
                // costs a GPU readback.
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.renderer.request_pick(phys_x, phys_y);
                }
            } else {
                self.cursor_in_viewport = None;
                // No picks are requested off-viewport, so drop the stale
                // hover state instead of letting the highlight linger.
                self.hovered_body = None;
                self.hovered_world_pos = None;
            }
        }

        let vp_cursor = self.cursor_in_viewport.map(|p| Vec2::new(p.0, p.1));
        self.camera.set_cursor_viewport(vp_cursor);

        let zoom_wheel_over_viewport =
            matches!(event, WindowEvent::MouseWheel { .. }) && self.cursor_in_viewport.is_some();

        if let Some(gfx) = self.gfx.as_mut() {
            let response = gfx.ui_layer.on_window_event(&gfx.window, &event);
            if response.repaint {
                gfx.window.request_redraw();
            }
            // egui-winit marks MouseWheel consumed when `wants_pointer_input()` — true over most
            // of the central panel — which prevented the CAD camera from ever seeing scroll.
            if response.consumed && !zoom_wheel_over_viewport {
                return;
            }
        }

        use winit::keyboard::Key;
        if let WindowEvent::KeyboardInput { event: ke, .. } = &event {
            if matches!(ke.state, ElementState::Pressed) {
                if let Key::Character(ch) = &ke.logical_key {
                    let s = ch.as_str();
                    if matches!(s, "h" | "H")
                        && self.cursor_in_viewport.is_some()
                        && self.camera.pivot_from_key_h(&self.user_settings.camera)
                    {
                        if let Some(gfx) = self.gfx.as_ref() {
                            gfx.window.request_redraw();
                        }
                    }
                    // Undo/redo. egui gets the event first, so typing in a
                    // text field never reaches here.
                    if self.modifiers.control_key() {
                        match s {
                            "z" | "Z" if self.modifiers.shift_key() => self.perform_redo(),
                            "z" => self.perform_undo(),
                            "y" | "Y" => self.perform_redo(),
                            _ => {}
                        }
                    }
                }
            }
        }

        if let WindowEvent::MouseInput {
            button: MouseButton::Middle,
            state: ElementState::Pressed,
            ..
        } = &event
        {
            if self.cursor_in_viewport.is_some() {
                let hit = self.hovered_world_pos.map(Vec3::from_array);
                self.camera
                    .on_mmb_pivot_pick(hit, &self.user_settings.camera);
                if let Some(gfx) = self.gfx.as_ref() {
                    gfx.window.request_redraw();
                }
            }
        }

        let wb = self.dispatch_workbench_input_without_select(&event);
        let mut redraw = wb.redraw;
        if wb.consumed {
            if redraw {
                if let Some(gfx) = self.gfx.as_ref() {
                    gfx.window.request_redraw();
                }
            }
            return;
        }

        let orbit_pick = self.hovered_world_pos.map(Vec3::from_array);
        let cam_res =
            self.camera
                .on_viewport_pointer(&event, &self.user_settings.camera, orbit_pick);
        redraw |= cam_res.wants_redraw();
        if matches!(cam_res, CameraPointerResult::LmbReleasedMaybeSelect) {
            redraw |= self.toggle_body_under_cursor_selection();
        }

        if redraw {
            if let Some(gfx) = self.gfx.as_ref() {
                gfx.window.request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                if self.confirm_discard_or_save() {
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.renderer.resize(size);
                }
                self.camera
                    .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
            }
            WindowEvent::ScaleFactorChanged {
                mut inner_size_writer,
                ..
            } => {
                if let Some(gfx) = self.gfx.as_mut() {
                    let size = gfx.window.inner_size();
                    let _ = inner_size_writer.request_inner_size(size);
                    gfx.renderer.resize(size);
                    self.camera
                        .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
                }
            }
            _ => {}
        }
    }

    fn dispatch_workbench_input_without_select(
        &mut self,
        event: &WindowEvent,
    ) -> core_document::InputResult {
        let wb_event = match self.convert_to_wb_event(event) {
            Some(e) => e,
            None => return core_document::InputResult::ignored(),
        };

        let wb_id = self.active_workbench_id();
        let active_tool_id = self.active_tool.active_ids.iter().next().cloned();
        let active_tool_str = active_tool_id.as_deref();
        let result = self.call_workbench_input(&wb_id, &wb_event, active_tool_str);

        if let Some(tool_id) = active_tool_id {
            if tool_id == "sketch.create" && result.consumed {
                self.active_tool.active_ids.remove(&tool_id);
            }
        }

        result
    }

    /// Call on_input on a workbench.
    fn call_workbench_input(
        &mut self,
        wb_id: &WorkbenchId,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
    ) -> core_document::InputResult {
        // Workbenches project the cursor themselves when no geometry is
        // hovered (e.g. wb_sketch casts onto its own sketch plane via
        // `WorkbenchRuntimeContext::viewport_to_plane`).
        let params = self.interaction_ctx_params();

        match self.with_workbench_ctx(wb_id, params, |wb, ctx| {
            wb.on_input(event, active_tool, ctx)
        }) {
            Some((result, outcome)) => {
                self.apply_hook_outcome(outcome);
                result
            }
            None => core_document::InputResult::ignored(),
        }
    }

    /// Convert a winit WindowEvent to a WorkbenchInputEvent.
    fn convert_to_wb_event(&self, event: &WindowEvent) -> Option<WorkbenchInputEvent> {
        match event {
            WindowEvent::MouseInput { state, button, .. } => {
                let wb_button = match button {
                    MouseButton::Left => WbMouseButton::Left,
                    MouseButton::Middle => WbMouseButton::Middle,
                    MouseButton::Right => WbMouseButton::Right,
                    MouseButton::Other(n) => WbMouseButton::Other(*n),
                    _ => return None,
                };
                let viewport_pos = self.cursor_in_viewport.unwrap_or((0.0, 0.0));
                match state {
                    ElementState::Pressed => Some(WorkbenchInputEvent::MousePress {
                        button: wb_button,
                        viewport_pos,
                    }),
                    ElementState::Released => Some(WorkbenchInputEvent::MouseRelease {
                        button: wb_button,
                        viewport_pos,
                    }),
                }
            }
            WindowEvent::CursorMoved { .. } => {
                let viewport_pos = self.cursor_in_viewport?;
                Some(WorkbenchInputEvent::MouseMove { viewport_pos })
            }
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{Key, NamedKey};
                let key = match &event.logical_key {
                    Key::Named(NamedKey::Escape) => core_document::KeyCode::Escape,
                    Key::Named(NamedKey::Enter) => core_document::KeyCode::Enter,
                    Key::Named(NamedKey::Space) => core_document::KeyCode::Space,
                    Key::Named(NamedKey::Delete) => core_document::KeyCode::Delete,
                    Key::Named(NamedKey::Backspace) => core_document::KeyCode::Backspace,
                    Key::Named(NamedKey::Tab) => core_document::KeyCode::Tab,
                    Key::Character(c) => match c.as_str() {
                        "a" | "A" => core_document::KeyCode::A,
                        "b" | "B" => core_document::KeyCode::B,
                        "c" | "C" => core_document::KeyCode::C,
                        "d" | "D" => core_document::KeyCode::D,
                        "e" | "E" => core_document::KeyCode::E,
                        "f" | "F" => core_document::KeyCode::F,
                        "g" | "G" => core_document::KeyCode::G,
                        "h" | "H" => core_document::KeyCode::H,
                        "i" | "I" => core_document::KeyCode::I,
                        "j" | "J" => core_document::KeyCode::J,
                        "k" | "K" => core_document::KeyCode::K,
                        "l" | "L" => core_document::KeyCode::L,
                        "m" | "M" => core_document::KeyCode::M,
                        "n" | "N" => core_document::KeyCode::N,
                        "o" | "O" => core_document::KeyCode::O,
                        "p" | "P" => core_document::KeyCode::P,
                        "q" | "Q" => core_document::KeyCode::Q,
                        "r" | "R" => core_document::KeyCode::R,
                        "s" | "S" => core_document::KeyCode::S,
                        "t" | "T" => core_document::KeyCode::T,
                        "u" | "U" => core_document::KeyCode::U,
                        "v" | "V" => core_document::KeyCode::V,
                        "w" | "W" => core_document::KeyCode::W,
                        "x" | "X" => core_document::KeyCode::X,
                        "y" | "Y" => core_document::KeyCode::Y,
                        "z" | "Z" => core_document::KeyCode::Z,
                        "0" => core_document::KeyCode::Key0,
                        "1" => core_document::KeyCode::Key1,
                        "2" => core_document::KeyCode::Key2,
                        "3" => core_document::KeyCode::Key3,
                        "4" => core_document::KeyCode::Key4,
                        "5" => core_document::KeyCode::Key5,
                        "6" => core_document::KeyCode::Key6,
                        "7" => core_document::KeyCode::Key7,
                        "8" => core_document::KeyCode::Key8,
                        "9" => core_document::KeyCode::Key9,
                        _ => core_document::KeyCode::Unknown,
                    },
                    _ => core_document::KeyCode::Unknown,
                };
                match event.state {
                    ElementState::Pressed => Some(WorkbenchInputEvent::KeyPress { key }),
                    ElementState::Released => Some(WorkbenchInputEvent::KeyRelease { key }),
                }
            }
            _ => None,
        }
    }

    fn toggle_body_under_cursor_selection(&mut self) -> bool {
        if let Some(hovered) = self.hovered_body {
            if self.selected_body == Some(hovered) {
                self.selected_body = None;
                app_log::info("Deselected body");
            } else {
                self.selected_body = Some(hovered);
                app_log::info(format!("Selected body: {hovered:?}"));
            }
        } else if self.selected_body.is_some() {
            self.selected_body = None;
            app_log::info("Deselected (clicked empty space)");
        }
        true
    }
}
