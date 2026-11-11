//! Window-event handling and workbench input dispatch.

use core_document::mesh_query::ray_triangle;
use core_document::{MouseButton as WbMouseButton, WorkbenchId, WorkbenchInputEvent};
use glam::{Vec2, Vec3};
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use render_wgpu::RenderBackend;
use std::time::Instant;
use uuid::Uuid;

use crate::PrintCadApp;
use crate::camera::CameraPointerResult;
use crate::log_panel as app_log;

pub(crate) use super::face_pick::*;

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

        // Real user input restarts the render-on-demand tail: frames keep
        // coming briefly after the last event, then the loop sleeps.
        if matches!(
            event,
            WindowEvent::CursorMoved { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::KeyboardInput { .. }
                | WindowEvent::ModifiersChanged(..)
                | WindowEvent::Touch(..)
                | WindowEvent::PinchGesture { .. }
                | WindowEvent::Resized(..)
                | WindowEvent::ScaleFactorChanged { .. }
                | WindowEvent::Focused(..)
                | WindowEvent::CursorEntered { .. }
                | WindowEvent::CursorLeft { .. }
                | WindowEvent::DroppedFile(..)
                | WindowEvent::HoveredFile(..)
        ) {
            self.last_input_time = Some(std::time::Instant::now());
        }
        // An OS-driven redraw (expose, resize damage) must render once.
        if matches!(event, WindowEvent::RedrawRequested) {
            self.redraw_needed = true;
        }

        // Track modifiers and pressed-button count regardless of who consumes
        // the event: the undo system uses "no button held" as its snapshot
        // boundary, and a release swallowed by egui must still decrement.
        // A release that ends a box selection is not also a click.
        let mut boxed = false;
        match &event {
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
                self.session.camera.set_modifiers(self.modifiers);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.mouse_buttons_down = self.mouse_buttons_down.saturating_add(1);
                    }
                    ElementState::Released => {
                        self.mouse_buttons_down = self.mouse_buttons_down.saturating_sub(1);
                    }
                }
                let at = match button {
                    MouseButton::Left => Some(0),
                    MouseButton::Right => Some(1),
                    MouseButton::Middle => Some(2),
                    _ => None,
                };
                if let Some(at) = at {
                    self.held_buttons[at] = state.is_pressed();
                }
                // The box ends with its button, whoever takes the release.
                if *button == MouseButton::Left
                    && !state.is_pressed()
                    && let Some(drag) = self.box_select.take()
                {
                    let threshold = self.user_settings.camera.click_drag_threshold_px
                        * self
                            .gfx
                            .as_ref()
                            .map_or(1.0, |g| g.window.scale_factor() as f32);
                    if drag.dragged(threshold) {
                        self.finish_box_select(drag);
                        boxed = true;
                    }
                    self.redraw_needed = true;
                }
            }
            // egui turns the paste key into the clipboard's text and drops
            // it when there is none; a chat reads a picture from it then.
            WindowEvent::KeyboardInput { event: key, .. }
                if key.state.is_pressed() && !key.repeat && is_paste_key(self.modifiers, key) =>
            {
                self.paste_pressed = true;
            }
            WindowEvent::Focused(true) => self.window_focused = true,
            // A release outside the window never arrives.
            WindowEvent::Focused(false) => {
                self.window_focused = false;
                self.box_select = None;
                self.mouse_buttons_down = 0;
                self.held_buttons = [false; 3];
                self.session.camera.forget_released([false; 3]);
                if let Some(side) = self.session.side_camera.as_mut() {
                    side.forget_released([false; 3]);
                }
                self.side_gesture = false;
                self.cancel_handle_drags();
                let wb_id = self.active_workbench_id();
                let params = self.interaction_ctx_params();
                if let Some(((), outcome)) = self.with_workbench_ctx(&wb_id, params, |wb, ctx| {
                    wb.cancel_pointer_gesture(ctx);
                }) {
                    self.apply_hook_outcome(
                        outcome,
                        crate::app::workbench_host::HookSite::Interaction,
                    );
                    self.redraw_needed = true;
                }
            }
            _ => {}
        }
        if self.cursor_in_viewport.is_some()
            && matches!(
                event,
                WindowEvent::MouseWheel { .. }
                    | WindowEvent::MouseInput {
                        state: ElementState::Pressed,
                        ..
                    }
            )
        {
            self.session.fit_on_layout = false;
        }
        if matches!(
            event,
            WindowEvent::CursorMoved { .. }
                | WindowEvent::Focused(false)
                | WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                }
        ) {
            self.session.camera.forget_released(self.held_buttons);
            if let Some(side) = self.session.side_camera.as_mut() {
                side.forget_released(self.held_buttons);
            }
        }

        // Picking and the viewport-local cursor update before egui sees the
        // event: egui can mark cursor events consumed while a UI drag runs,
        // and 3D hit testing and zoom-to-focal-plane math still need
        // consistent coordinates.
        // A modal, menu or tooltip drawn over the viewport owns the pointer:
        // the scene must neither hover nor zoom under it.
        let floating_ui_owns_pointer = self
            .gfx
            .as_ref()
            .is_some_and(|gfx| gfx.ui_layer.pointer_over_floating_ui());

        if let WindowEvent::CursorMoved { position, .. } = &event {
            // `CursorMoved` is already [`PhysicalPosition`]; match renderer + viewport_rect.
            let phys_x = position.x.max(0.0).round() as u32;
            let phys_y = position.y.max(0.0).round() as u32;

            let vp = self.session.camera.viewport_info();
            let cursor_x = phys_x as f32 - vp.0;
            let cursor_y = phys_y as f32 - vp.1;

            if self.session.mark_gesture.is_some() {
                self.mark_move(Vec2::new(cursor_x, cursor_y));
            }
            // The box's corner follows the pointer, held to the view.
            if let Some(drag) = self.box_select.as_mut() {
                drag.to = Vec2::new(
                    cursor_x.clamp(0.0, vp.2 as f32),
                    cursor_y.clamp(0.0, vp.3 as f32),
                );
                self.redraw_needed = true;
            }

            // The split view's side pane: a cursor over it hovers and picks
            // what it shows, with its camera.
            self.cursor_in_side = self.session.side_rect.and_then(|(x, y, w, h)| {
                let (sx, sy) = (phys_x as f32 - x as f32, phys_y as f32 - y as f32);
                let inside = sx >= 0.0 && sy >= 0.0 && sx < w as f32 && sy < h as f32;
                (self.side_gesture || (inside && !floating_ui_owns_pointer)).then_some((sx, sy))
            });
            if self.cursor_in_side.is_some() {
                self.cursor_in_viewport = None;
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.renderer.request_pick(phys_x, phys_y);
                }
                self.session.hovered_feature = None;
                self.session.hovered_part = None;
                self.session.hovered_edge = None;
            } else if cursor_x >= 0.0
                && cursor_y >= 0.0
                && cursor_x < vp.2 as f32
                && cursor_y < vp.3 as f32
                && !floating_ui_owns_pointer
            {
                self.cursor_in_viewport = Some((cursor_x, cursor_y));
                // Only pick while the cursor is over the 3D viewport; over
                // egui panels the hover state is irrelevant and each pick
                // costs a GPU readback.
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.renderer.request_pick(phys_x, phys_y);
                }
                // Sketch hover feedback (CPU hit-test; the GPU pick can't
                // reliably hit hairline sketch curves). Skipped while
                // editing: the sketcher renders its own hover state.
                self.session.hovered_feature = if self.sketch_editing_active() {
                    None
                } else {
                    self.feature_under_cursor()
                };
                self.session.hovered_part = self.part_under_cursor();
            } else {
                self.cursor_in_viewport = None;
                // No picks are requested off-viewport, so drop the stale
                // hover state instead of letting the highlight linger.
                self.session.hovered_body = None;
                self.session.hovered_world_pos = None;
                self.session.hovered_triangle = None;
                self.session.hovered_edge = None;
                self.session.hovered_part = None;
            }
        }

        let vp_cursor = self.cursor_in_viewport.map(|p| Vec2::new(p.0, p.1));
        self.session.camera.set_cursor_viewport(vp_cursor);

        let zoom_wheel_over_viewport = matches!(
            event,
            WindowEvent::MouseWheel { .. } | WindowEvent::PinchGesture { .. }
        ) && (self.cursor_in_viewport.is_some()
            || self.cursor_in_side.is_some())
            && !floating_ui_owns_pointer;

        if let Some(gfx) = self.gfx.as_mut() {
            let response = gfx.ui_layer.on_window_event(&gfx.window, &event);
            // egui answers `repaint: true` to RedrawRequested itself;
            // re-requesting there would chain compositor frame callbacks into
            // a display-rate loop that never idles. That event is satisfied
            // by the frame this wake is about to render.
            if response.repaint && !matches!(event, WindowEvent::RedrawRequested) {
                self.redraw_needed = true;
                gfx.window.request_redraw();
            }
            // egui-winit marks MouseWheel consumed whenever `wants_pointer_input()`
            // is true, which is most of the central panel, so the camera would
            // never see scroll.
            // It also marks Tab consumed unconditionally; keys belong to the
            // workbench whenever no text field owns the keyboard.
            let key_for_workbench = matches!(event, WindowEvent::KeyboardInput { .. })
                && !gfx.ui_layer.wants_keyboard_input();
            if response.consumed && !zoom_wheel_over_viewport && !key_for_workbench {
                return;
            }
            // A question in the window keeps the keys: no bench or tool
            // takes Escape from under its Cancel.
            if matches!(event, WindowEvent::KeyboardInput { .. })
                && (self.unsaved_asking.is_some()
                    || self.save_over_asking.is_some()
                    || self.session.mesh_units_pending.is_some())
            {
                return;
            }
            // A press on the viewport is on no widget: egui would keep the
            // last text field focused and swallow every key after it.
            if matches!(
                event,
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                }
            ) && self.cursor_in_viewport.is_some()
            {
                gfx.ui_layer.release_focus();
            }
        }

        use winit::keyboard::Key;
        // A floating card takes Enter and Escape for its OK and Cancel, and
        // a press in the view elsewhere is its OK: the card closes before
        // the press goes on, and while OK is held back the press does
        // nothing.
        if self.session.card.is_some() {
            if let WindowEvent::KeyboardInput { event: ke, .. } = &event
                && matches!(
                    ke.logical_key,
                    Key::Named(
                        winit::keyboard::NamedKey::Escape | winit::keyboard::NamedKey::Enter
                    )
                )
            {
                self.redraw_needed = true;
                return;
            }
            if matches!(
                event,
                WindowEvent::MouseInput {
                    state: ElementState::Pressed,
                    ..
                }
            ) && self.cursor_in_viewport.is_some()
                && !self.close_card(true)
            {
                return;
            }
        }
        // Escape puts the measure tool away before anything else sees it.
        if let WindowEvent::KeyboardInput { event: ke, .. } = &event
            && matches!(ke.state, ElementState::Pressed)
            && matches!(
                ke.logical_key,
                Key::Named(winit::keyboard::NamedKey::Escape)
            )
            && self.session.measure.take().is_some()
        {
            self.redraw_needed = true;
            return;
        }
        // Then the note card, then the note tool.
        if let WindowEvent::KeyboardInput { event: ke, .. } = &event
            && matches!(ke.state, ElementState::Pressed)
            && matches!(
                ke.logical_key,
                Key::Named(winit::keyboard::NamedKey::Escape)
            )
            && (self.session.note_card.take().is_some()
                || std::mem::take(&mut self.session.note_tool))
        {
            self.redraw_needed = true;
            return;
        }
        if self.side_pane_input(&event, boxed) {
            return;
        }
        // The array task's spacing handle and the clipping plane's, before
        // a bench or the camera.
        if self.array_handle_input(&event) || self.section_input(&event) {
            if let Some(gfx) = self.gfx.as_ref() {
                gfx.window.request_redraw();
            }
            return;
        }
        let text = self.typed_text_of(&event);
        let mut wb = self.dispatch_workbench_input_without_select(&event);
        if let Some(text) = text {
            let wb_id = self.active_workbench_id();
            let active_tool = self.session.active_tool.active_ids.iter().next().cloned();
            let typed = self.call_workbench_input(
                &wb_id,
                &WorkbenchInputEvent::Text { text },
                active_tool.as_deref(),
            );
            wb.consumed |= typed.consumed;
            wb.redraw |= typed.redraw;
        }
        let mut redraw = wb.redraw;
        // Escape the bench left alone leaves the isolated view.
        if !wb.consumed
            && let WindowEvent::KeyboardInput { event: ke, .. } = &event
            && matches!(ke.state, ElementState::Pressed)
            && ke.logical_key == Key::Named(winit::keyboard::NamedKey::Escape)
            && self.session.document.isolation().is_some()
        {
            self.session.document.set_isolation(None);
            redraw = true;
        }
        // A key the workbench consumed must not also reach egui's widgets
        // (Tab would move focus, Enter would accept the open task).
        if wb.consumed
            && let WindowEvent::KeyboardInput { event: ke, .. } = &event
            && let winit::keyboard::Key::Named(named) = &ke.logical_key
            && let Some(gfx) = self.gfx.as_mut()
        {
            use winit::keyboard::NamedKey;
            let key = match named {
                NamedKey::Tab => Some(egui::Key::Tab),
                NamedKey::Enter => Some(egui::Key::Enter),
                NamedKey::Escape => Some(egui::Key::Escape),
                NamedKey::Delete => Some(egui::Key::Delete),
                NamedKey::Backspace => Some(egui::Key::Backspace),
                _ => None,
            };
            if let Some(key) = key {
                gfx.ui_layer.swallow_key(key);
            }
        }
        if wb.consumed {
            if redraw && let Some(gfx) = self.gfx.as_ref() {
                gfx.window.request_redraw();
            }
            return;
        }

        // While a marking tool is in hand, a drag over the view marks the
        // scan (the right button or Alt rubs out); Shift leaves it to the
        // camera.
        if self.session.mark_arm.is_some()
            && let WindowEvent::MouseInput { button, state, .. } = &event
            && matches!(button, MouseButton::Left | MouseButton::Right)
        {
            if state.is_pressed()
                && !self.modifiers.shift_key()
                && let Some((x, y)) = self.cursor_in_viewport
            {
                let erase = *button == MouseButton::Right || self.modifiers.alt_key();
                self.mark_press(Vec2::new(x, y), erase);
                return;
            }
            if !state.is_pressed() && self.session.mark_gesture.is_some() {
                self.mark_release();
                if let Some(gfx) = self.gfx.as_ref() {
                    gfx.window.request_redraw();
                }
                return;
            }
        }

        // A left press no bench took, on empty space, starts a box.
        if matches!(
            event,
            WindowEvent::MouseInput {
                button: MouseButton::Left,
                state: ElementState::Pressed,
                ..
            }
        ) && let Some((x, y)) = self.cursor_in_viewport
            && self.may_start_box()
        {
            self.box_select = Some(crate::app::box_select::BoxDrag::at(Vec2::new(x, y)));
        }

        // A right press held still opens the radial menu; letting go
        // starts the slice pointed at, and is no click for the view.
        let cursor = self.cursor_in_viewport.map(|(x, y)| Vec2::new(x, y));
        let mut radial_took = false;
        match &event {
            WindowEvent::MouseInput {
                button: MouseButton::Right,
                state: ElementState::Pressed,
                ..
            } => {
                if let Some(cursor) = cursor {
                    self.radial_pressed(cursor);
                }
            }
            WindowEvent::MouseInput {
                button: MouseButton::Right,
                state: ElementState::Released,
                ..
            } => radial_took = self.radial_released(event_loop),
            WindowEvent::CursorMoved { .. } => {
                if let Some(cursor) = cursor {
                    self.radial_moved(cursor);
                }
            }
            _ => {}
        }

        let orbit_pick = self.session.hovered_world_pos.map(Vec3::from_array);
        let cam_res =
            self.session
                .camera
                .on_viewport_pointer(&event, &self.user_settings.camera, orbit_pick);
        redraw |= cam_res.wants_redraw();
        if matches!(cam_res, CameraPointerResult::LmbReleasedMaybeSelect) && !boxed {
            redraw |= if self.session.section_pick.is_some() {
                self.section_pick_click()
            } else if self.session.measure.is_some() {
                self.measure_click()
            } else if self.session.note_tool {
                self.note_click()
            } else {
                self.toggle_body_under_cursor_selection()
            };
        }
        if matches!(cam_res, CameraPointerResult::RmbReleasedMaybeMenu) && !radial_took {
            redraw |= self.open_viewport_menu();
        }

        if redraw && let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }

        match event {
            WindowEvent::CloseRequested => self.request_quit(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.renderer.resize(size);
                }
                self.session
                    .camera
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
                    self.session
                        .camera
                        .update_viewport((0, 0), (size.width.max(1), size.height.max(1)));
                }
            }
            _ => {}
        }
    }

    /// Pointer input over the split view's side pane, or in a gesture
    /// begun there. The pane stands in for the viewport while it is
    /// handled: its camera is the session's (the viewport's own while the
    /// cameras are linked, the pane being the viewport's size) and the
    /// cursor is in its pixels, so the bench, the camera and a click's
    /// selection all read what the side pane's pick found. Whether it took
    /// the event.
    fn side_pane_input(&mut self, event: &WindowEvent, boxed: bool) -> bool {
        let pointer = matches!(
            event,
            WindowEvent::CursorMoved { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::PinchGesture { .. }
        );
        if !pointer || self.session.side_rect.is_none() {
            self.side_gesture = false;
            return false;
        }
        let pressed = matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
        );
        if pressed && self.cursor_in_side.is_some() && self.mouse_buttons_down == 1 {
            self.side_gesture = true;
        }
        if !self.side_gesture && (self.cursor_in_side.is_none() || self.mouse_buttons_down > 0) {
            return false;
        }
        let viewport_cursor = std::mem::replace(&mut self.cursor_in_viewport, self.cursor_in_side);
        let main_camera = self
            .session
            .side_camera
            .take()
            .map(|side| std::mem::replace(&mut self.session.camera, side));
        let redraw = self.side_pane_turn(event, boxed);
        if let Some(main) = main_camera {
            self.session.side_camera = Some(std::mem::replace(&mut self.session.camera, main));
        }
        self.cursor_in_viewport = viewport_cursor;
        if self.mouse_buttons_down == 0 {
            self.side_gesture = false;
        }
        if redraw && let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }
        true
    }

    /// [`Self::side_pane_input`]'s event, the side pane standing in for the
    /// viewport. Whether to draw again.
    fn side_pane_turn(&mut self, event: &WindowEvent, boxed: bool) -> bool {
        let wb = self.dispatch_workbench_input_without_select(event);
        if wb.consumed {
            return wb.redraw;
        }
        let cursor = self.cursor_in_viewport.map(|(x, y)| Vec2::new(x, y));
        let orbit_pick = self.session.hovered_world_pos.map(Vec3::from_array);
        let camera = &mut self.session.camera;
        camera.set_modifiers(self.modifiers);
        camera.set_cursor_viewport(cursor);
        let result = camera.on_viewport_pointer(event, &self.user_settings.camera, orbit_pick);
        let mut redraw = wb.redraw || result.wants_redraw();
        if matches!(result, CameraPointerResult::LmbReleasedMaybeSelect) && !boxed {
            redraw |= if self.session.section_pick.is_some() {
                self.section_pick_click()
            } else if self.session.measure.is_some() {
                self.measure_click()
            } else if self.session.note_tool {
                self.note_click()
            } else {
                self.toggle_body_under_cursor_selection()
            };
        }
        redraw
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
        let active_tool_id = self.session.active_tool.active_ids.iter().next().cloned();
        let active_tool_str = active_tool_id.as_deref();
        let result = self.call_workbench_input(&wb_id, &wb_event, active_tool_str);

        // Action-behaviour tools fire once: consume them as soon as the
        // workbench handled an event with them active.
        if let Some(tool_id) = active_tool_id
            && result.consumed
            && self.tool_is_action(&wb_id, &tool_id)
        {
            self.session.active_tool.active_ids.remove(&tool_id);
        }

        result
    }

    /// Hand every active Action tool to the workbench now. The toolbar only
    /// sets the id; without this the tool would sit there highlighted until
    /// some unrelated event reached the workbench.
    pub(crate) fn dispatch_activated_tools(&mut self) {
        let wb_id = self.active_workbench_id();
        let pending: Vec<String> = self
            .session
            .active_tool
            .active_ids
            .iter()
            .filter(|id| self.tool_is_action(&wb_id, id))
            .cloned()
            .collect();
        for tool_id in pending {
            let result = self.call_workbench_input(
                &wb_id,
                &WorkbenchInputEvent::ToolActivated,
                Some(&tool_id),
            );
            if result.consumed {
                self.session.active_tool.active_ids.remove(&tool_id);
            }
            if result.redraw || result.consumed {
                self.redraw_needed = true;
            }
        }
    }

    fn tool_is_action(&self, wb_id: &WorkbenchId, tool_id: &str) -> bool {
        self.registry
            .tools_for(wb_id)
            .map(|tools| {
                tools.iter().any(|t| {
                    t.id == core_document::base_tool_id(tool_id)
                        && t.behavior == core_document::ToolBehavior::Action
                })
            })
            .unwrap_or(false)
    }

    /// Run a workbench's keyboard action, bound to a key, with its active
    /// tool, when that workbench is still the active one.
    pub(crate) fn run_bench_action(&mut self, workbench: &WorkbenchId, id: &str) {
        if *workbench != self.active_workbench_id() {
            return;
        }
        let active_tool = self.session.active_tool.active_ids.iter().next().cloned();
        let event = WorkbenchInputEvent::Action { id: id.to_string() };
        let result = self.call_workbench_input(workbench, &event, active_tool.as_deref());
        if result.redraw || result.consumed {
            self.redraw_needed = true;
        }
    }

    pub(crate) fn call_workbench_input(
        &mut self,
        wb_id: &WorkbenchId,
        event: &WorkbenchInputEvent,
        active_tool: Option<&str>,
    ) -> core_document::InputResult {
        // Workbenches project the cursor themselves when no geometry is
        // hovered (the sketcher casts onto its own sketch plane via
        // `WorkbenchRuntimeContext::viewport_to_plane`).
        let params = self.interaction_ctx_params();

        match self.with_workbench_ctx(wb_id, params, |wb, ctx| {
            wb.on_input(event, active_tool, ctx)
        }) {
            Some((result, outcome)) => {
                self.apply_hook_outcome(outcome, crate::app::workbench_host::HookSite::Interaction);
                result
            }
            None => core_document::InputResult::ignored(),
        }
    }

    /// The text a key press types for the active workbench, while it takes
    /// typed values and no text field has the keyboard: what the key types,
    /// unless it runs a shortcut and the workbench is not yet in the middle
    /// of a value; for Ctrl+V, what the clipboard holds.
    fn typed_text_of(&mut self, event: &WindowEvent) -> Option<String> {
        let WindowEvent::KeyboardInput { event: ke, .. } = event else {
            return None;
        };
        if ke.state != ElementState::Pressed {
            return None;
        }
        let wb_id = self.active_workbench_id();
        let (numeric, mid_value) = self
            .registry
            .workbench(&wb_id)
            .map(|wb| (wb.takes_numeric_input(), wb.takes_text_input()))
            .unwrap_or_default();
        let gfx = self.gfx.as_mut()?;
        if !numeric || gfx.ui_layer.wants_keyboard_input() {
            return None;
        }
        let ctrl = self.modifiers.control_key() || self.modifiers.super_key();
        let alt = self.modifiers.alt_key();
        let key = key_code_of(&ke.logical_key);
        if ctrl && !alt && key == core_document::KeyCode::V {
            return gfx.ui_layer.clipboard_text();
        }
        if ctrl || alt {
            return None;
        }
        let text = ke.text.as_ref()?.to_string();
        if text.is_empty() || text.chars().any(char::is_control) {
            return None;
        }
        let chord = core_document::Chord {
            ctrl: false,
            shift: self.modifiers.shift_key(),
            alt: false,
            key,
        };
        let shortcut =
            !crate::ui::keymap::types_number(chord) && gfx.ui_layer.is_shortcut(chord, &wb_id);
        (mid_value || !shortcut).then_some(text)
    }

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
                let key = key_code_of(&event.logical_key);
                match event.state {
                    ElementState::Pressed => Some(WorkbenchInputEvent::KeyPress { key }),
                    ElementState::Released => Some(WorkbenchInputEvent::KeyRelease { key }),
                }
            }
            _ => None,
        }
    }

    /// The unit the measure tool reads in.
    pub(crate) fn measure_unit(&self) -> core_document::Unit {
        crate::app::measure::readout_unit(&self.session.document)
    }

    /// A click with the measure tool armed: the edge under the cursor, else
    /// a datum, else the face, else the point, joins the chain of picks; a
    /// click on nothing starts over.
    fn measure_click(&mut self) -> bool {
        use crate::app::measure::{MeasurePick, pick_at, readout};
        let edge = self.session.hovered_edge.map(|edge| MeasurePick::Edge {
            point: edge.point,
            direction: edge.direction,
            length: edge.length_mm,
            circle: edge.circle,
            body: Some(core_document::BodyId(edge.body)),
        });
        let datum = edge.is_none().then(|| self.datum_under_cursor()).flatten();
        let mesh = self.session.hovered_body.and_then(|body| {
            self.session
                .document
                .imported_geometry(core_document::BodyId(body))
        });
        let pick = datum.or_else(|| {
            pick_at(
                edge,
                mesh.map(|geometry| &*geometry.mesh),
                self.session.hovered_world_pos,
            )
        });
        let unit = self.measure_unit();
        let Some(picks) = self.session.measure.as_mut() else {
            return false;
        };
        let Some(mut pick) = pick else {
            // Empty space: the chain starts over.
            let had = !picks.is_empty();
            picks.clear();
            self.session.measure_gap = None;
            return had;
        };
        if let MeasurePick::Face { body, .. } = &mut pick {
            *body = self.session.hovered_body.map(core_document::BodyId);
        }
        picks.push(pick);
        app_log::info(format!("Measured {}", readout(picks, unit).join(", ")));
        self.ask_measure_gap();
        true
    }

    /// The datum under the cursor, placed in the world, as a measure pick.
    pub(crate) fn datum_under_cursor(&self) -> Option<crate::app::measure::MeasurePick> {
        let id = self.session.hovered_feature?;
        let document = &self.session.document;
        let placed = document.placed_datum(id)?;
        let (eye, ray) = self.cursor_ray()?;
        let (datum, point) =
            crate::app::measure::DatumPick::of(&placed.feature, placed.placement, eye, ray);
        Some(crate::app::measure::MeasurePick::Datum {
            name: placed.name,
            datum,
            point,
        })
    }

    /// With the last two picks faces or edges of solids, ask the kernel for
    /// the least distance between them; otherwise drop the last answer.
    fn ask_measure_gap(&mut self) {
        let picks = self.session.measure.clone().unwrap_or_default();
        self.session.measure_gap = self.ask_gap(&picks);
    }

    /// The status bar's quick measure of the selection, a line each: asked
    /// again of the kernel only when the selection changes. Empty while it
    /// is turned off or nothing is picked.
    pub(crate) fn drive_quick_measure(&mut self) -> Vec<String> {
        if !self.user_settings.status_bar.quick_measure {
            self.session.quick_measure = None;
            return Vec::new();
        }
        let picks = self.selection_picks();
        if picks.is_empty() {
            self.session.quick_measure = None;
            return Vec::new();
        }
        if self
            .session
            .quick_measure
            .as_ref()
            .is_none_or(|q| q.picks != picks)
        {
            let gap = self.ask_gap(&picks);
            self.session.quick_measure = Some(crate::app::measure::QuickMeasure { picks, gap });
        }
        let unit = self.session.document.display_unit();
        self.session
            .quick_measure
            .as_ref()
            .map(|q| q.lines(unit))
            .unwrap_or_default()
    }

    /// The faces and edges picked in the view as measure picks, in world
    /// space: the faces of the selected body in the order picked, then the
    /// edges.
    fn selection_picks(&self) -> Vec<crate::app::measure::MeasurePick> {
        use crate::app::measure::{MeasurePick, mesh_area};
        let mut picks = Vec::new();
        if let Some((body, _)) = self
            .session
            .last_face_hit
            .filter(|(body, _)| self.session.selected_body == Some(*body))
        {
            let last = self
                .session
                .last_face_hit
                .zip(self.session.face_highlight.as_ref())
                .map(|((_, face), highlight)| (face, highlight));
            let faces = self
                .session
                .earlier_faces
                .iter()
                .filter(|p| p.highlight.body == body)
                .map(|p| (p.face, &p.highlight))
                .chain(last);
            for (face, highlight) in faces {
                picks.push(MeasurePick::Face {
                    point: face.point,
                    normal: face.normal,
                    surface: face.surface,
                    area: mesh_area(&highlight.mesh),
                    body: Some(core_document::BodyId(body)),
                });
            }
        }
        for edge in &self.session.selected_edges {
            picks.push(MeasurePick::Edge {
                point: edge.point,
                direction: edge.direction,
                length: edge.length_mm,
                circle: edge.circle,
                body: Some(core_document::BodyId(edge.body)),
            });
        }
        picks
    }

    /// With the last two of `picks` faces or edges of solids, ask the
    /// kernel for the least distance between them, under a token of its
    /// own.
    fn ask_gap(
        &mut self,
        picks: &[crate::app::measure::MeasurePick],
    ) -> Option<crate::app::measure::MeasureGap> {
        use crate::app::measure::{MeasureGap, gap_operands};
        let [(a, part_a, pa), (b, part_b, pb)] = gap_operands(picks)?;
        let document = &self.session.document;
        let (Some(first), Some(second)) = (
            document.imported_brep_blob_arc(a),
            document.imported_brep_blob_arc(b),
        ) else {
            return None;
        };
        self.session.gap_token += 1;
        let token = self.session.gap_token;
        let (place_a, place_b) = (document.body_placement(a), document.body_placement(b));
        let local = |placement: core_document::BodyPlacement, p: [f32; 3]| {
            placement.inverse().point(p).map(f64::from)
        };
        let relative = place_a.inverse().after(&place_b);
        let m = glam::Mat4::from_rotation_translation(relative.quat(), relative.offset());
        let rows = m
            .transpose()
            .to_cols_array_2d()
            .map(|row| row.map(f64::from));
        self.kernel_worker
            .request_min_distance(crate::kernel_worker::KernelRequest::MinDistance {
                body_id: a.0,
                token,
                first,
                first_part: part_a,
                first_near: local(place_a, pa),
                second,
                second_part: part_b,
                second_near: local(place_b, pb),
                second_in_first: rows,
            });
        Some(MeasureGap {
            token,
            first_placement: place_a,
            found: None,
        })
    }

    /// A right click that did not pan: over a body, the menu for that body
    /// opens where the pointer is; anywhere else it closes whatever was open.
    fn open_viewport_menu(&mut self) -> bool {
        let Some(gfx) = self.gfx.as_ref() else {
            return false;
        };
        let body = self.session.hovered_body.filter(|id| {
            // Sketch feature meshes pick as bodies too; they have no body menu.
            self.session
                .document
                .get_feature_meta(core_document::FeatureId(*id))
                .is_none()
        });
        let Some((cx, cy)) = self.cursor_in_viewport else {
            self.session.viewport_menu = None;
            return true;
        };
        let vp = self.session.camera.viewport_info();
        let scale = gfx.window.scale_factor() as f32;
        let face = self
            .session
            .hovered_face
            .as_ref()
            .filter(|hover| Some(hover.body) == body)
            .map(|hover| hover.face);
        let repeat = self.last_tool_label();
        let point = self.session.hovered_world_pos.filter(|_| body.is_some());
        let others = if body.is_some() {
            self.faces_under_cursor()
        } else {
            Vec::new()
        };
        self.session.viewport_menu = body.map(|body| crate::ui::ViewportMenu {
            body: core_document::BodyId(body),
            at: [(vp.0 + cx) / scale, (vp.1 + cy) / scale],
            face,
            point,
            repeat,
            others,
        });
        true
    }

    /// The ray from the eye through the cursor on the viewport, in world
    /// space: its origin and unit direction. `None` off the viewport.
    pub(crate) fn cursor_ray(&self) -> Option<(Vec3, Vec3)> {
        let cursor = self.cursor_in_viewport?;
        let camera = &self.session.camera;
        let vp = camera.viewport_info();
        let (origin, direction) = core_document::runtime::viewport_to_ray(
            camera.view_projection(),
            (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            cursor,
        )?;
        Some((Vec3::from_array(origin), Vec3::from_array(direction)))
    }

    /// Every face the cursor's ray crosses, front to back, over every
    /// visible body that takes picks, each face once: what Select other
    /// lists. A face the clipping plane hides is left out.
    pub(crate) fn faces_under_cursor(&self) -> Vec<crate::ui::OtherPick> {
        let Some((origin, direction)) = self.cursor_ray() else {
            return Vec::new();
        };
        let document = &self.session.document;
        // Each face's nearest crossing: (distance, body, face, triangle).
        let mut crossings: Vec<(f32, Uuid, u32, u32)> = Vec::new();
        for (body, geometry) in document.imported_geometries() {
            if !document.imported_body_effective_visible(*body)
                || !document.body_selectable(*body)
                || document.is_mesh_body(*body)
                || document
                    .get_feature_meta(core_document::FeatureId(body.0))
                    .is_some()
            {
                continue;
            }
            let clip = self.clip_of(*body);
            let mesh = &*geometry.mesh;
            if mesh.faces.len() != mesh.indices.len() / 3 {
                continue;
            }
            if let Some((lo, hi)) = geometry.bounds()
                && !ray_meets_box(origin, direction, lo, hi)
            {
                continue;
            }
            let corner = |i: u32| Vec3::from_array(mesh.positions[i as usize]);
            for (index, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
                let Some(t) = ray_triangle(
                    origin,
                    direction,
                    [corner(tri[0]), corner(tri[1]), corner(tri[2])],
                ) else {
                    continue;
                };
                if clip.is_some_and(|plane| {
                    !crate::camera::section::keeps(plane, origin + direction * t)
                }) {
                    continue;
                }
                let face = mesh.faces[index];
                match crossings.iter_mut().find(|c| c.1 == body.0 && c.2 == face) {
                    Some(c) if t < c.0 => *c = (t, body.0, face, index as u32),
                    Some(_) => {}
                    None => crossings.push((t, body.0, face, index as u32)),
                }
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        let nearest = crossings.first().map_or(0.0, |c| c.0);
        crossings
            .into_iter()
            .take(OTHER_PICKS)
            .map(|(distance, body, face, triangle)| {
                let id = core_document::BodyId(body);
                let name = document
                    .body(id)
                    .map_or_else(|| "Body".to_string(), |b| b.name.clone());
                let kind = document
                    .imported_geometry(id)
                    .and_then(|g| g.mesh.face_surfaces.get(face as usize).copied())
                    .map_or("face", surface_word);
                crate::ui::OtherPick {
                    body: id,
                    triangle,
                    // How far behind the nearest face it lies tells alike
                    // faces apart.
                    label: if distance - nearest < 1e-3 {
                        format!("{name} · {kind}")
                    } else {
                        format!("{name} · {kind}, {:.1} mm behind", distance - nearest)
                    },
                }
            })
            .collect()
    }

    /// Select the face of `body` its mesh's `triangle` lies on, as a plain
    /// click on it would: Select other's choice.
    pub(crate) fn select_other(&mut self, body: core_document::BodyId, triangle: u32) {
        let Some(geometry) = self.session.document.imported_geometry(body) else {
            return;
        };
        let mesh = std::sync::Arc::clone(&geometry.mesh);
        let revision = geometry.revision;
        let Some(tri) = mesh.indices.as_chunks::<3>().0.get(triangle as usize) else {
            return;
        };
        let centre = tri
            .iter()
            .map(|i| Vec3::from_array(mesh.positions[*i as usize]))
            .sum::<Vec3>()
            / 3.0;
        let pick = Some(HoveredTriangle {
            index: triangle,
            revision,
        });
        let Some(face) = face_ref_at_pick(&mesh, revision, pick, centre) else {
            return;
        };
        let highlight =
            face_submesh(&mesh, revision, pick, face.point, face.normal).map(|(id, submesh)| {
                FaceHighlight {
                    body: body.0,
                    face: id,
                    mesh: std::sync::Arc::new(submesh),
                    revision: self
                        .session
                        .face_highlight
                        .as_ref()
                        .map_or(0, |f| f.revision.wrapping_add(1)),
                }
            });
        self.session.selected_edges.clear();
        self.session.earlier_faces.clear();
        self.session.earlier_bodies.clear();
        self.session.selected_body = Some(body.0);
        self.session.last_face_hit = Some((body.0, face));
        self.session.face_highlight = highlight;
        self.session.last_select_click = None;
        app_log::info("Selected face");
    }

    /// Drop what is picked in the view: the body, its faces and its edges.
    pub(crate) fn clear_view_selection(&mut self) {
        self.session.selected_body = None;
        self.session.last_face_hit = None;
        self.session.earlier_faces.clear();
        self.session.earlier_bodies.clear();
        self.session.face_highlight = None;
        self.session.selected_edges.clear();
        self.session.picked_parts = None;
        self.session.last_select_click = None;
    }

    pub(crate) fn toggle_body_under_cursor_selection(&mut self) -> bool {
        // A click picks one body at most; the others a box took go.
        self.session.earlier_bodies.clear();
        // A region of a closed sketch takes the click on its own, and Ctrl
        // takes a curve of one beside the regions.
        let ctrl = self.modifiers.control_key();
        if let Some((feature_id, part)) = self.part_under_cursor()
            && (ctrl || matches!(part, core_document::FeaturePart::Region(_)))
        {
            self.pick_part(feature_id, part, ctrl);
            return true;
        }
        // Sketch curves and datums first: their lines are far too thin for
        // the 1-pixel GPU pick to hit reliably, so clicks are matched
        // against them on the CPU with a proper pixel tolerance.
        if let Some(feature_id) = self.feature_under_cursor() {
            self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature_id));
            self.session.last_face_hit = None;
            self.session.face_highlight = None;
            app_log::info(format!("Selected feature {feature_id:?}"));
            return true;
        }

        // An edge under the cursor takes the click before the face it
        // borders; Ctrl adds it to the picked edges, a plain click replaces
        // them.
        if let Some(hit) = self.session.hovered_edge {
            let ctrl = self.modifiers.control_key();
            // A double click on an edge takes every edge that runs on from
            // it smoothly: a fillet's or chamfer's round outline in one go.
            let now = Instant::now();
            let double = self.session.last_edge_click.is_some_and(|(t, body, edge)| {
                body == hit.body && edge == hit.edge && now.duration_since(t).as_millis() < 400
            });
            self.session.last_edge_click = Some((now, hit.body, hit.edge));
            if double
                && let Some(geometry) = self
                    .session
                    .document
                    .imported_geometry(core_document::BodyId(hit.body))
            {
                let mesh = &geometry.mesh;
                let chain: Vec<crate::app::edges::EdgeHit> =
                    crate::app::edges::tangent_chain(mesh, hit.edge)
                        .into_iter()
                        .filter_map(|edge| crate::app::edges::edge_hit(mesh, hit.body, edge))
                        .collect();
                if !ctrl {
                    self.session.selected_edges.clear();
                }
                for edge in chain {
                    if !self
                        .session
                        .selected_edges
                        .iter()
                        .any(|s| s.body == edge.body && s.edge == edge.edge)
                    {
                        self.session.selected_edges.push(edge);
                    }
                }
                if !ctrl || self.session.selected_body != Some(hit.body) {
                    self.session.face_highlight = None;
                    self.session.last_face_hit = None;
                }
                self.session.selected_body = Some(hit.body);
                app_log::info(format!(
                    "Selected {} edge(s) along the chain",
                    self.session.selected_edges.len()
                ));
                return true;
            }
            let already = self
                .session
                .selected_edges
                .iter()
                .position(|s| s.body == hit.body && s.edge == hit.edge);
            match (ctrl, already) {
                (true, Some(i)) => {
                    self.session.selected_edges.remove(i);
                }
                (true, None) => self.session.selected_edges.push(hit),
                (false, _) => self.session.selected_edges = vec![hit],
            }
            // Ctrl keeps the faces picked on the same body beside the edges.
            if !ctrl || self.session.selected_body != Some(hit.body) {
                self.session.face_highlight = None;
                self.session.last_face_hit = None;
            }
            self.session.selected_body = Some(hit.body);
            app_log::info(format!(
                "Selected {} edge(s)",
                self.session.selected_edges.len()
            ));
            return true;
        }

        let filter = self.session.pick_filter;
        if let Some(hovered) = self.session.hovered_body {
            // Picking only edges, a click on a body away from them takes
            // nothing.
            if filter == crate::ui::PickFilter::Edges {
                return true;
            }
            // A feature's own geometry is occasionally GPU-picked too (e.g.
            // clicking exactly on a sketch line): same selection path.
            let feature_id = core_document::FeatureId(hovered);
            if self.session.document.get_feature_meta(feature_id).is_some() {
                self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature_id));
                self.session.last_face_hit = None;
                return true;
            }

            // Face-first selection: the first click selects the FACE under the
            // cursor; a double click promotes to the whole body. A mesh has
            // no faces to pick, so a click on one takes the whole body.
            let now = Instant::now();
            let previous = self.session.last_select_click;
            // Ctrl picks faces one after another, however quick.
            let is_double = !self.modifiers.control_key()
                && previous
                    .map(|(t, target)| target == hovered && now.duration_since(t).as_millis() < 400)
                    .unwrap_or(false);
            // Whether the last viewport click also landed on this body: a
            // selection made from the tree or by an import is not something
            // the next click should undo.
            let clicked_before = previous.is_some_and(|(_, target)| target == hovered);
            self.session.last_select_click = Some((now, hovered));
            let already_whole = self.session.selected_body == Some(hovered)
                && self.session.face_highlight.is_none()
                && clicked_before;
            let whole_mesh = self
                .session
                .document
                .is_mesh_body(core_document::BodyId(hovered))
                && !already_whole;

            let whole_only = filter == crate::ui::PickFilter::Bodies;
            // A double click on a face of a body built from features opens
            // the feature that made it, as a double click on its row would.
            let maker = (is_double
                && !whole_only
                && !self.registry.is_modal(&self.session.active_workbench.0))
            .then(|| self.face_hit_under_cursor(hovered))
            .flatten()
            .and_then(|face| {
                self.session
                    .document
                    .face_maker(core_document::BodyId(hovered), face.name)
            });
            if let Some(feature) = maker {
                self.apply_tree_activation(crate::ui::TreeItemId::Feature(feature));
                app_log::info("Editing the feature that made the face");
            } else if is_double || whole_mesh || (whole_only && !already_whole) {
                // The whole body the face belongs to: one part of an
                // assembly, not the assembly. A modelling bench works from
                // the tree, so there the body's row opens and scrolls into
                // view; an edit session keeps the tree still.
                self.session.face_highlight = None;
                self.session.selected_body = Some(hovered);
                if !self.registry.is_modal(&self.session.active_workbench.0) {
                    self.session.reveal_body = Some(core_document::BodyId(hovered));
                }
                app_log::info(format!("Selected body: {hovered:?}"));
            } else if self.session.selected_body == Some(hovered)
                && self.session.face_highlight.is_none()
                && clicked_before
            {
                // Clicking an already fully-selected body deselects it.
                self.session.selected_body = None;
                self.session.last_face_hit = None;
                app_log::info("Deselected body");
            } else {
                let ctrl = self.modifiers.control_key();
                // A face pick without Ctrl lets the edges go.
                if !ctrl {
                    self.session.selected_edges.clear();
                }
                let hit = self.face_hit_under_cursor(hovered);
                let highlight = hit.and_then(|face| self.face_highlight_of(hovered, face));
                // Ctrl adds a face of the body already picked on, or takes
                // a picked one out; otherwise the pick starts again.
                let adding = ctrl
                    && self.session.selected_body == Some(hovered)
                    && self
                        .session
                        .last_face_hit
                        .is_some_and(|(b, _)| b == hovered)
                    && self.session.face_highlight.is_some();
                self.session.selected_body = Some(hovered);
                match (adding, hit, highlight) {
                    (true, Some(face), Some(highlight)) => self.toggle_face(face, highlight),
                    (_, hit, highlight) => {
                        self.session.earlier_faces.clear();
                        self.session.last_face_hit = hit.map(|face| (hovered, face));
                        self.session.face_highlight = highlight;
                    }
                }
                self.session.reveal_face_maker =
                    !self.registry.is_modal(&self.session.active_workbench.0);
                let picked = self.selected_face_refs().len();
                if picked > 1 {
                    app_log::info(format!("Selected {picked} faces"));
                } else {
                    app_log::info(
                        "Selected face (Ctrl adds more; double-click edits the feature that made it, or takes the whole body)",
                    );
                }
            }
        } else if self.session.selected_body.is_some()
            || !self.session.selected_edges.is_empty()
            || self.session.picked_parts.is_some()
        {
            self.clear_view_selection();
            app_log::info("Deselected (clicked empty space)");
        }
        true
    }

    /// The highlight of `face` of `body`: the triangles it is drawn with.
    pub(crate) fn face_highlight_of(
        &self,
        body: Uuid,
        face: core_document::FaceRef,
    ) -> Option<FaceHighlight> {
        let geometry = self
            .session
            .document
            .imported_geometry(core_document::BodyId(body))?;
        let (id, submesh) = face_submesh(
            &geometry.mesh,
            geometry.revision,
            self.hovered_triangle_of(body),
            face.point,
            face.normal,
        )?;
        let revision = self
            .session
            .face_highlight
            .as_ref()
            .map(|f| f.revision.wrapping_add(1))
            .unwrap_or(0);
        Some(FaceHighlight {
            body,
            face: id,
            mesh: std::sync::Arc::new(submesh),
            revision,
        })
    }

    /// Ctrl's face pick: a face already picked is taken out, any other is
    /// added as the last ([`toggled`]).
    fn toggle_face(&mut self, face: core_document::FaceRef, highlight: FaceHighlight) {
        let mut picked: Vec<PickedFace> = std::mem::take(&mut self.session.earlier_faces);
        if let (Some((_, last)), Some(last_highlight)) = (
            self.session.last_face_hit,
            self.session.face_highlight.take(),
        ) {
            picked.push(PickedFace {
                face: last,
                highlight: last_highlight,
            });
        }
        let mut picked = toggled(picked, PickedFace { face, highlight });
        let last = picked.pop();
        self.session.earlier_faces = picked;
        self.session.last_face_hit = last.as_ref().map(|p| (p.highlight.body, p.face));
        self.session.face_highlight = last.map(|p| p.highlight);
    }

    /// The visible feature whose geometry passes within a few pixels of the
    /// cursor, as its bench measures it (a sketch: unprojected onto its
    /// plane and hit-tested in sketch coordinates). Hidden features are
    /// skipped; the nearest hit wins.
    pub(crate) fn feature_under_cursor(&self) -> Option<core_document::FeatureId> {
        let pick = self.viewport_pick()?;
        self.registry
            .pick_feature(&self.session.document, &pick, FEATURE_PICK_PX)
    }

    /// The cursor and the view it sits in, for picking features.
    fn viewport_pick(&self) -> Option<core_document::ViewportPick> {
        let cursor = self.cursor_in_viewport?;
        let vp = self.session.camera.viewport_info();
        Some(core_document::ViewportPick {
            view_proj: self.session.camera.view_projection(),
            viewport: (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            cursor,
        })
    }

    /// The part of a visible feature under the cursor while no task or
    /// edit session is open: a curve within reach before a region, then
    /// the nearest the view, unless a body the pick found stands in front.
    pub(crate) fn part_under_cursor(
        &self,
    ) -> Option<(core_document::FeatureId, core_document::FeaturePart)> {
        if self.session.task_open || self.sketch_editing_active() {
            return None;
        }
        let pick = self.viewport_pick()?;
        let eye = Vec3::from_array(self.session.camera.position());
        let forward = (Vec3::from_array(self.session.camera.target()) - eye).normalize_or_zero();
        let depth = |p: [f32; 3]| (Vec3::from_array(p) - eye).dot(forward);
        let document = &self.session.document;
        // The body's depth read on the face under the cursor, its plane
        // there met by the cursor's ray: the pick's point is only as exact
        // as the depth buffer.
        let ray =
            core_document::runtime::viewport_to_ray(pick.view_proj, pick.viewport, pick.cursor);
        let body_depth =
            self.session
                .hovered_body
                .filter(|b| {
                    document
                        .get_feature_meta(core_document::FeatureId(*b))
                        .is_none()
                })
                .and_then(|body| {
                    let on_face = self.face_ref_under_cursor(body).zip(ray).and_then(
                        |(face, (origin, dir))| {
                            let (point, normal) =
                                (Vec3::from_array(face.point), Vec3::from_array(face.normal));
                            let (origin, dir) = (Vec3::from_array(origin), Vec3::from_array(dir));
                            let facing = dir.dot(normal);
                            (facing.abs() > 1e-6)
                                .then(|| origin + dir * ((point - origin).dot(normal) / facing))
                        },
                    );
                    on_face
                        .map(|p| p.to_array())
                        .or(self.session.hovered_world_pos)
                })
                .map(depth);
        let curve_last = |part: &core_document::FeaturePart| {
            !matches!(part, core_document::FeaturePart::Edge(_))
        };
        self.registry
            .parts_under_cursor(document, &pick, FEATURE_PICK_PX)
            .into_iter()
            .filter(|(_, hit)| body_depth.is_none_or(|d| depth(hit.point) <= d + PART_IN_FRONT_MM))
            .min_by(|(_, a), (_, b)| {
                curve_last(&a.part)
                    .cmp(&curve_last(&b.part))
                    .then(depth(a.point).total_cmp(&depth(b.point)))
            })
            .map(|(id, hit)| (id, hit.part))
    }

    /// Pick `part` of `feature` on its own: alone, or with `adding` beside
    /// the parts of it picked already, or taken out when it is one.
    /// Taking out the last leaves the whole feature selected.
    fn pick_part(
        &mut self,
        feature: core_document::FeatureId,
        part: core_document::FeaturePart,
        adding: bool,
    ) {
        let mut parts = self
            .session
            .picked_parts
            .take()
            .filter(|(f, _)| adding && *f == feature)
            .filter(|_| self.session.active_document_object == Some(feature))
            .map(|(_, parts)| parts)
            .unwrap_or_default();
        match parts.iter().position(|p| *p == part) {
            Some(i) if adding => {
                parts.remove(i);
            }
            Some(_) => {}
            None => parts.push(part),
        }
        if self.session.active_document_object != Some(feature)
            || self.session.tree_selection != Some(crate::ui::TreeItemId::Feature(feature))
        {
            self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature));
        }
        self.session.last_face_hit = None;
        self.session.face_highlight = None;
        let regions = parts
            .iter()
            .filter(|p| matches!(p, core_document::FeaturePart::Region(_)))
            .count();
        let curves = parts.len() - regions;
        app_log::info(match (regions, curves) {
            (0, 0) => "Selected the whole feature".to_string(),
            (r, 0) => format!("Selected {r} region(s) (Ctrl adds more)"),
            (0, c) => format!("Selected {c} curve(s)"),
            (r, c) => format!("Selected {r} region(s), {c} curve(s)"),
        });
        self.session.picked_parts = (!parts.is_empty()).then_some((feature, parts));
    }

    /// Derive the face (surface point + normal) under the cursor from the
    /// picked body's mesh: the triangle the pick pass drew there, else the
    /// one nearest the picked point.
    fn face_hit_under_cursor(&self, body: Uuid) -> Option<core_document::FaceRef> {
        let point = glam::Vec3::from_array(self.session.hovered_world_pos?);
        let geometry = self
            .session
            .document
            .imported_geometry(core_document::BodyId(body))?;
        face_ref_at_pick(
            &geometry.mesh,
            geometry.revision,
            self.hovered_triangle_of(body),
            point,
        )
    }

    /// The face under the cursor on `body`, as benches are told of it:
    /// from the triangle the pick drew there, or, on a mesh small enough to
    /// search every frame, the triangle nearest the picked point.
    pub(crate) fn face_ref_under_cursor(&self, body: Uuid) -> Option<core_document::FaceRef> {
        let point = glam::Vec3::from_array(self.session.hovered_world_pos?);
        let geometry = self
            .session
            .document
            .imported_geometry(core_document::BodyId(body))?;
        let pick = self.hovered_triangle_of(body);
        if pick.is_none_or(|p| p.revision != geometry.revision)
            && geometry.mesh.indices.len() / 3 > HOVER_SEARCH_TRIANGLES
        {
            return None;
        }
        face_ref_at_pick(&geometry.mesh, geometry.revision, pick, point)
    }

    /// The triangle the pick drew under the cursor, when `body` is the one
    /// it found there.
    pub(crate) fn hovered_triangle_of(&self, body: Uuid) -> Option<HoveredTriangle> {
        self.session
            .hovered_triangle
            .filter(|_| self.session.hovered_body == Some(body))
    }
}

/// How near, in pixels, the cursor must come to a feature's lines to be
/// over it.
const FEATURE_PICK_PX: f32 = 8.0;

/// How far, in mm, a region may lie behind the face the pick found and
/// still take the cursor: a sketch drawn on a face lies on it.
const PART_IN_FRONT_MM: f32 = 0.05;

/// The most triangles a body may have for the face under the cursor to be
/// searched for each frame when the pick pass names no triangle.
const HOVER_SEARCH_TRIANGLES: usize = 50_000;

/// The most faces Select other lists.
const OTHER_PICKS: usize = 12;

/// A face's kind of surface in a word, for a menu.
/// The keys egui pastes with: Paste, Ctrl+V (Cmd+V on macOS, also by the
/// key's place for layouts without a V), Shift+Insert on Windows.
fn is_paste_key(modifiers: winit::keyboard::ModifiersState, key: &winit::event::KeyEvent) -> bool {
    use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};
    let command = match cfg!(target_os = "macos") {
        true => modifiers.super_key(),
        false => modifiers.control_key(),
    };
    let v = match &key.logical_key {
        Key::Character(c) => c.eq_ignore_ascii_case("v"),
        _ => key.physical_key == PhysicalKey::Code(KeyCode::KeyV),
    };
    key.logical_key == Key::Named(NamedKey::Paste)
        || (command && v)
        || (cfg!(windows)
            && modifiers.shift_key()
            && key.logical_key == Key::Named(NamedKey::Insert))
}

fn surface_word(surface: kernel_api::FaceSurface) -> &'static str {
    use kernel_api::FaceSurface as S;
    match surface {
        S::Plane { .. } => "plane",
        S::Cylinder { .. } => "cylinder",
        S::Cone { .. } => "cone",
        S::Sphere { .. } => "sphere",
        S::Torus { .. } => "torus",
        _ => "face",
    }
}

/// Whether the ray from `origin` in `direction` meets the box `lo`..`hi`.
fn ray_meets_box(origin: Vec3, direction: Vec3, lo: [f32; 3], hi: [f32; 3]) -> bool {
    let (mut near, mut far) = (0.0f32, f32::MAX);
    for axis in 0..3 {
        let d = direction[axis];
        if d.abs() < 1e-12 {
            if origin[axis] < lo[axis] || origin[axis] > hi[axis] {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo[axis] - origin[axis]) / d, (hi[axis] - origin[axis]) / d);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

/// The key a workbench is told of for a key the keyboard sent.
fn key_code_of(key: &winit::keyboard::Key) -> core_document::KeyCode {
    use winit::keyboard::{Key, NamedKey};
    match key {
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
            "." => core_document::KeyCode::Period,
            "," => core_document::KeyCode::Comma,
            "-" => core_document::KeyCode::Minus,
            _ => core_document::KeyCode::Unknown,
        },
        _ => core_document::KeyCode::Unknown,
    }
}

#[cfg(test)]
mod picking_faces {
    use super::*;

    /// The ray Select other casts finds a triangle in front of it at its
    /// distance, misses one beside it or behind it, and passes a box it
    /// does not meet.
    #[test]
    fn select_other_s_ray_meets_what_lies_across_it() {
        let tri = [
            Vec3::new(-1.0, -1.0, 5.0),
            Vec3::new(1.0, -1.0, 5.0),
            Vec3::new(0.0, 1.0, 5.0),
        ];
        let ahead = Vec3::Z;
        assert_eq!(ray_triangle(Vec3::ZERO, ahead, tri), Some(5.0));
        assert_eq!(
            ray_triangle(Vec3::new(3.0, 0.0, 0.0), ahead, tri),
            None,
            "beside it"
        );
        assert_eq!(ray_triangle(Vec3::ZERO, -ahead, tri), None, "behind");
        assert!(ray_meets_box(
            Vec3::ZERO,
            ahead,
            [-1.0, -1.0, 4.0],
            [1.0, 1.0, 6.0]
        ));
        assert!(!ray_meets_box(
            Vec3::new(3.0, 0.0, 0.0),
            ahead,
            [-1.0, -1.0, 4.0],
            [1.0, 1.0, 6.0]
        ));
    }

    fn face(body: Uuid, id: u32) -> PickedFace {
        PickedFace {
            face: core_document::FaceRef {
                point: [id as f32, 0.0, 0.0],
                normal: [0.0, 0.0, 1.0],
                surface: None,
                name: 0,
            },
            highlight: FaceHighlight {
                body,
                face: Some(id),
                mesh: Default::default(),
                revision: 0,
            },
        }
    }

    fn ids(picked: &[PickedFace]) -> Vec<u32> {
        picked.iter().filter_map(|p| p.highlight.face).collect()
    }

    #[test]
    fn ctrl_adds_new_faces_in_order_and_takes_a_picked_one_out() {
        let body = Uuid::new_v4();
        let picked = toggled(vec![face(body, 1)], face(body, 2));
        let picked = toggled(picked, face(body, 3));
        assert_eq!(ids(&picked), [1, 2, 3]);
        let picked = toggled(picked, face(body, 2));
        assert_eq!(ids(&picked), [1, 3], "picked again, it goes");
        let picked = toggled(picked, face(body, 3));
        assert_eq!(
            ids(&picked),
            [1],
            "the last going leaves the one before it last"
        );
    }

    #[test]
    fn a_face_with_no_kernel_id_is_always_added() {
        let body = Uuid::new_v4();
        let mut bare = face(body, 7);
        bare.highlight.face = None;
        let picked = toggled(vec![bare.clone()], bare);
        assert_eq!(picked.len(), 2);
    }
}
