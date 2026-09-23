//! Window-event handling and workbench input dispatch.

use core_document::{MouseButton as WbMouseButton, WorkbenchId, WorkbenchInputEvent};
use glam::{Vec2, Vec3};
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use render_vk::RenderBackend;
use std::time::Instant;
use uuid::Uuid;

use crate::PrintCadApp;
use crate::camera::CameraPointerResult;
use crate::log_panel as app_log;

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

            if cursor_x >= 0.0
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
                // editing — the sketcher renders its own hover state.
                self.session.hovered_feature = if self.sketch_editing_active() {
                    None
                } else {
                    self.feature_under_cursor()
                };
            } else {
                self.cursor_in_viewport = None;
                // No picks are requested off-viewport, so drop the stale
                // hover state instead of letting the highlight linger.
                self.session.hovered_body = None;
                self.session.hovered_world_pos = None;
                self.session.hovered_edge = None;
            }
        }

        let vp_cursor = self.cursor_in_viewport.map(|p| Vec2::new(p.0, p.1));
        self.session.camera.set_cursor_viewport(vp_cursor);

        let zoom_wheel_over_viewport = matches!(event, WindowEvent::MouseWheel { .. })
            && self.cursor_in_viewport.is_some()
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
            // egui-winit marks MouseWheel consumed when `wants_pointer_input()` — true over most
            // of the central panel — which prevented the CAD camera from ever seeing scroll.
            // It also marks Tab consumed unconditionally; keys belong to the
            // workbench whenever no text field owns the keyboard.
            let key_for_workbench = matches!(event, WindowEvent::KeyboardInput { .. })
                && !gfx.ui_layer.wants_keyboard_input();
            if response.consumed && !zoom_wheel_over_viewport && !key_for_workbench {
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
        if let WindowEvent::KeyboardInput { event: ke, .. } = &event
            && matches!(ke.state, ElementState::Pressed)
            && let Key::Character(ch) = &ke.logical_key
        {
            let s = ch.as_str();
            if matches!(s, "h" | "H")
                && self.cursor_in_viewport.is_some()
                && self
                    .session
                    .camera
                    .pivot_from_key_h(&self.user_settings.camera)
                && let Some(gfx) = self.gfx.as_ref()
            {
                gfx.window.request_redraw();
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

        let wb = self.dispatch_workbench_input_without_select(&event);
        let mut redraw = wb.redraw;
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

        let orbit_pick = self.session.hovered_world_pos.map(Vec3::from_array);
        let cam_res =
            self.session
                .camera
                .on_viewport_pointer(&event, &self.user_settings.camera, orbit_pick);
        redraw |= cam_res.wants_redraw();
        if matches!(cam_res, CameraPointerResult::LmbReleasedMaybeSelect) {
            redraw |= if self.session.measure.is_some() {
                self.measure_click()
            } else {
                self.toggle_body_under_cursor_selection()
            };
        }
        if matches!(cam_res, CameraPointerResult::RmbReleasedMaybeMenu) {
            redraw |= self.open_viewport_menu();
        }

        if redraw && let Some(gfx) = self.gfx.as_ref() {
            gfx.window.request_redraw();
        }

        match event {
            WindowEvent::CloseRequested => {
                if self.confirm_close_all() {
                    // Let any queued write finish; exiting mid-file would
                    // leave a truncated document.
                    self.wait_for_all_document_saves();
                    event_loop.exit();
                }
            }
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

    /// Call on_input on a workbench.
    fn call_workbench_input(
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
                        "." => core_document::KeyCode::Period,
                        "," => core_document::KeyCode::Comma,
                        "-" => core_document::KeyCode::Minus,
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

    /// A click with the measure tool armed: the point under the cursor
    /// joins the measurement; a third click starts over.
    fn measure_click(&mut self) -> bool {
        // An edge under the cursor snaps the pick onto it.
        let Some(point) = self
            .session
            .hovered_edge
            .map(|e| e.point)
            .or(self.session.hovered_world_pos)
        else {
            return false;
        };
        let unit = self.session.document.display_unit();
        let Some(points) = self.session.measure.as_mut() else {
            return false;
        };
        if points.len() >= 2 {
            points.clear();
        }
        points.push(point);
        if let [a, b] = points.as_slice() {
            let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
            let fmt = |v: f32| core_document::format_length_mm(v, unit, 2);
            app_log::info(format!(
                "Measured {} (Δx {} Δy {} Δz {})",
                fmt(d),
                fmt((b[0] - a[0]).abs()),
                fmt((b[1] - a[1]).abs()),
                fmt((b[2] - a[2]).abs()),
            ));
        }
        true
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
        self.session.viewport_menu = body.map(|body| crate::ui::ViewportMenu {
            body: core_document::BodyId(body),
            at: [(vp.0 + cx) / scale, (vp.1 + cy) / scale],
        });
        true
    }

    fn toggle_body_under_cursor_selection(&mut self) -> bool {
        // Sketch curves first: their tessellated lines are far too thin for
        // the 1-pixel GPU pick to hit reliably, so clicks are matched
        // against sketch geometry on the CPU with a proper pixel tolerance.
        if let Some(feature_id) = self.feature_under_cursor() {
            self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature_id));
            self.session.last_face_hit = None;
            self.session.face_highlight = None;
            app_log::info(format!("Selected sketch {feature_id:?}"));
            return true;
        }

        // An edge under the cursor takes the click before the face it
        // borders; Ctrl adds it to the picked edges, a plain click replaces
        // them.
        if let Some(hit) = self.session.hovered_edge {
            let ctrl = self.modifiers.control_key();
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
            self.session.face_highlight = None;
            self.session.last_face_hit = None;
            self.session.selected_body = Some(hit.body);
            app_log::info(format!(
                "Selected {} edge(s)",
                self.session.selected_edges.len()
            ));
            return true;
        }

        if let Some(hovered) = self.session.hovered_body {
            // A feature's own geometry is occasionally GPU-picked too (e.g.
            // clicking exactly on a sketch line): same selection path.
            let feature_id = core_document::FeatureId(hovered);
            if self.session.document.get_feature_meta(feature_id).is_some() {
                self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature_id));
                self.session.last_face_hit = None;
                return true;
            }

            // Face-first selection: the first click selects the FACE under the
            // cursor; a double click promotes to the whole body.
            let now = Instant::now();
            let previous = self.session.last_select_click;
            let is_double = previous
                .map(|(t, target)| target == hovered && now.duration_since(t).as_millis() < 400)
                .unwrap_or(false);
            // Whether the last viewport click also landed on this body: a
            // selection made from the tree or by an import is not something
            // the next click should undo.
            let clicked_before = previous.is_some_and(|(_, target)| target == hovered);
            self.session.last_select_click = Some((now, hovered));

            if is_double {
                // The whole body the face belongs to — one part of an
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
                // A face pick without Ctrl lets the edges go.
                if !self.modifiers.control_key() {
                    self.session.selected_edges.clear();
                }
                self.session.selected_body = Some(hovered);
                self.session.last_face_hit = self
                    .face_hit_under_cursor(hovered)
                    .map(|face| (hovered, face));
                self.session.face_highlight =
                    self.session.last_face_hit.and_then(|(body, face)| {
                        let geometry = self
                            .session
                            .document
                            .imported_geometry(core_document::BodyId(body))?;
                        let submesh = face_submesh(&geometry.mesh, face.point, face.normal)?;
                        let revision = self
                            .session
                            .face_highlight
                            .as_ref()
                            .map(|f| f.revision.wrapping_add(1))
                            .unwrap_or(0);
                        Some(FaceHighlight {
                            body,
                            mesh: std::sync::Arc::new(submesh),
                            revision,
                        })
                    });
                app_log::info("Selected face (double-click for the whole body)");
            }
        } else if self.session.selected_body.is_some() || !self.session.selected_edges.is_empty() {
            self.session.selected_body = None;
            self.session.last_face_hit = None;
            self.session.face_highlight = None;
            self.session.selected_edges.clear();
            self.session.last_select_click = None;
            app_log::info("Deselected (clicked empty space)");
        }
        true
    }

    /// The visible feature whose geometry passes within a few pixels of the
    /// cursor, as its bench measures it (a sketch: unprojected onto its
    /// plane and hit-tested in sketch coordinates). Hidden features are
    /// skipped; the nearest hit wins.
    fn feature_under_cursor(&self) -> Option<core_document::FeatureId> {
        const TOLERANCE_PX: f32 = 8.0;
        let cursor = self.cursor_in_viewport?;
        let vp = self.session.camera.viewport_info();
        let pick = core_document::ViewportPick {
            view_proj: self.session.camera.view_projection(),
            viewport: (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            cursor,
        };
        self.registry
            .pick_feature(&self.session.document, &pick, TOLERANCE_PX)
    }

    /// Derive the face (surface point + normal) under the cursor from the
    /// picked body's mesh. Runs only on selection clicks, so a linear scan
    /// is fine.
    fn face_hit_under_cursor(&self, body: Uuid) -> Option<core_document::FaceRef> {
        let point = glam::Vec3::from_array(self.session.hovered_world_pos?);
        let geometry = self
            .session
            .document
            .imported_geometry(core_document::BodyId(body))?;
        face_ref_from_mesh(&geometry.mesh, point)
    }
}

/// Resolve a picked world position to a face reference on `mesh`.
///
/// The GPU pick reconstructs the position from the depth buffer, whose
/// precision varies with view angle and distance — the raw point can sit a
/// millimetre or more off the surface, far enough that a naive containment
/// test misses the face and falls back to whole-body selection. So:
/// find the nearest triangle, take the face plane from ITS exact vertices,
/// and project the picked point onto that plane.
pub(crate) fn face_ref_from_mesh(
    mesh: &kernel_api::TriMesh,
    point: glam::Vec3,
) -> Option<core_document::FaceRef> {
    let (_, dist_sq, anchor, normal) = nearest_triangle(mesh, point)?;

    // Sanity bound relative to the model size: the pick already identified
    // this body, so the nearest triangle is the right face unless the
    // position is wildly stale.
    let (min, max) = mesh.bounds()?;
    let diag = (glam::Vec3::from_array(max) - glam::Vec3::from_array(min)).length();
    let limit = (diag * 0.05).max(1.0);
    if dist_sq > limit * limit {
        return None;
    }

    // Project the noisy picked point onto the triangle's exact plane so the
    // face plane (and any sketch placed on it) is depth-error free.
    let projected = point - normal * (point - anchor).dot(normal);
    Some(core_document::FaceRef {
        point: projected.to_array(),
        normal: normal.to_array(),
    })
}

/// The triangle of `mesh` nearest to `point`: its index, squared distance,
/// one vertex, and unit normal.
fn nearest_triangle(
    mesh: &kernel_api::TriMesh,
    point: glam::Vec3,
) -> Option<(usize, f32, glam::Vec3, glam::Vec3)> {
    let mut best: Option<(usize, f32, glam::Vec3, glam::Vec3)> = None;
    for (index, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let normal = (b - a).cross(c - a);
        if normal.length_squared() < 1e-12 {
            continue;
        }
        let d = point_triangle_distance_sq(point, a, b, c);
        if best.map(|(_, bd, _, _)| d < bd).unwrap_or(true) {
            best = Some((index, d, a, normal.normalize()));
        }
    }
    best
}

/// The sub-mesh of one face, for the selection highlight.
///
/// When the mesh knows which kernel face each triangle came from, the face
/// is the one under the hit, whole — a cylinder wall as much as a flat side.
/// A mesh without that (a sketch, a document saved before faces were
/// recorded) falls back to the plane through the hit, which is exact for a
/// flat face and one strip of a curved one.
pub(crate) fn face_submesh(
    mesh: &kernel_api::TriMesh,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<kernel_api::TriMesh> {
    if mesh.faces.len() == mesh.indices.len() / 3 && !mesh.faces.is_empty() {
        let hit = glam::Vec3::from_array(point);
        if let Some((tri, _, _, _)) = nearest_triangle(mesh, hit) {
            return face_submesh_by_id(mesh, mesh.faces[tri]);
        }
    }
    coplanar_face_submesh(mesh, point, normal)
}

/// Every triangle cut from kernel face `face`, each lifted along its own
/// normal so the highlight never z-fights the surface it covers.
pub(crate) fn face_submesh_by_id(
    mesh: &kernel_api::TriMesh,
    face: u32,
) -> Option<kernel_api::TriMesh> {
    const LIFT: f32 = 0.05;
    let mut out = kernel_api::TriMesh::default();
    for (tri, id) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.faces) {
        if *id != face {
            continue;
        }
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-12 {
            continue;
        }
        let n = n.normalize();
        let base = out.positions.len() as u32;
        for v in [a, b, c] {
            out.positions.push((v + n * LIFT).to_array());
            out.normals.push(n.to_array());
        }
        out.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    (!out.indices.is_empty()).then_some(out)
}

/// Sub-mesh rendered as the single-face selection highlight.
pub(crate) struct FaceHighlight {
    pub body: Uuid,
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// Bumped whenever the sub-mesh is re-extracted.
    pub revision: u64,
}

/// Every triangle of `mesh` lying on the plane (point, normal), lifted
/// slightly along it. The face as geometry sees it: exact for a flat face,
/// one strip of a curved one, and two flat faces on one plane together.
pub(crate) fn coplanar_face_submesh(
    mesh: &kernel_api::TriMesh,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<kernel_api::TriMesh> {
    const NORMAL_ALIGN: f32 = 0.999;
    const PLANE_TOL: f32 = 0.05;
    const LIFT: f32 = 0.05;
    let n = glam::Vec3::from_array(normal).normalize();
    let p0 = glam::Vec3::from_array(point);

    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for tri in mesh.indices.as_chunks::<3>().0 {
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let tri_n_raw = (b - a).cross(c - a);
        if tri_n_raw.length_squared() < 1e-12 {
            continue;
        }
        let tri_n = tri_n_raw.normalize();
        if tri_n.dot(n) < NORMAL_ALIGN {
            continue;
        }
        if (a - p0).dot(n).abs() > PLANE_TOL {
            continue;
        }
        let base = positions.len() as u32;
        for v in [a, b, c] {
            positions.push((v + n * LIFT).to_array());
            normals.push(n.to_array());
        }
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    if indices.is_empty() {
        return None;
    }
    Some(kernel_api::TriMesh {
        positions,
        normals,
        indices,
        edges: Vec::new(),
        colors: Vec::new(),
        faces: Vec::new(),
        edge_ids: Vec::new(),
    })
}

/// Squared distance from `p` to triangle `abc` (closest-point projection).
fn point_triangle_distance_sq(p: glam::Vec3, a: glam::Vec3, b: glam::Vec3, c: glam::Vec3) -> f32 {
    // Ericson, "Real-Time Collision Detection", closest point on triangle.
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length_squared();
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length_squared();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (ap - ab * v).length_squared();
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length_squared();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (ap - ac * w).length_squared();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (bp - (c - b) * w).length_squared();
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    (p - (a + ab * v + ac * w)).length_squared()
}

#[cfg(test)]
mod tests {
    use super::coplanar_face_submesh;
    use kernel_api::TriMesh;

    /// Two quads: top face at z=1 (normal +Z), bottom at z=0 (normal -Z).
    fn two_face_mesh() -> TriMesh {
        let positions = vec![
            // top (CCW seen from +Z)
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0],
            [0.0, 1.0, 1.0],
            // bottom (CCW seen from -Z)
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
        ];
        let indices = vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7];
        TriMesh {
            positions,
            normals: Vec::new(),
            indices,
            edges: Vec::new(),
            colors: Vec::new(),
            faces: Vec::new(),
            edge_ids: Vec::new(),
        }
    }

    #[test]
    fn extracts_only_the_hit_plane_and_lifts_it() {
        let mesh = two_face_mesh();
        let sub = coplanar_face_submesh(&mesh, [0.5, 0.5, 1.0], [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(sub.indices.len(), 6, "only the top quad's two triangles");
        assert!(
            sub.positions.iter().all(|p| (p[2] - 1.05).abs() < 1e-4),
            "lifted 0.05 along the normal"
        );
    }

    #[test]
    fn noisy_pick_point_still_resolves_the_face() {
        use super::face_ref_from_mesh;
        let mesh = two_face_mesh();
        // Simulate depth-buffer error: 0.4 above the top face.
        let face = face_ref_from_mesh(&mesh, glam::Vec3::new(0.5, 0.5, 1.4)).unwrap();
        assert!((glam::Vec3::from_array(face.normal) - glam::Vec3::Z).length() < 1e-4);
        assert!(
            (face.point[2] - 1.0).abs() < 1e-4,
            "point projected onto the exact face plane: {:?}",
            face.point
        );
        // And extraction from the projected point succeeds.
        let sub = coplanar_face_submesh(&mesh, face.point, face.normal).unwrap();
        assert_eq!(sub.indices.len(), 6);
    }

    #[test]
    fn wildly_stale_point_is_rejected() {
        use super::face_ref_from_mesh;
        let mesh = two_face_mesh();
        assert!(face_ref_from_mesh(&mesh, glam::Vec3::new(50.0, 50.0, 50.0)).is_none());
    }

    /// A quarter-cylinder wall as three strips that turn 30° each, all cut
    /// from one kernel face, next to one flat strip from another.
    fn curved_face_mesh() -> TriMesh {
        let mut mesh = TriMesh::default();
        let mut strip = |angle_a: f32, angle_b: f32, face: u32| {
            let (sa, ca) = angle_a.to_radians().sin_cos();
            let (sb, cb) = angle_b.to_radians().sin_cos();
            let base = mesh.positions.len() as u32;
            mesh.positions
                .extend([[ca, sa, 0.0], [cb, sb, 0.0], [cb, sb, 1.0], [ca, sa, 1.0]]);
            mesh.indices
                .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            mesh.faces.extend([face, face]);
        };
        strip(0.0, 30.0, 7);
        strip(30.0, 60.0, 7);
        strip(60.0, 90.0, 7);
        strip(90.0, 120.0, 8);
        mesh
    }

    #[test]
    fn a_curved_face_selects_whole_when_the_mesh_knows_its_faces() {
        use super::face_submesh;
        let mesh = curved_face_mesh();
        // A hit on the middle strip, with that strip's normal.
        let hit = [45f32.to_radians().cos(), 45f32.to_radians().sin(), 0.5];
        let normal = [hit[0], hit[1], 0.0];
        let sub = face_submesh(&mesh, hit, normal).unwrap();
        assert_eq!(
            sub.indices.len(),
            3 * 6,
            "all three strips of face 7, not the fourth"
        );
    }

    #[test]
    fn without_face_ids_the_plane_selects_one_strip() {
        use super::face_submesh;
        let mut mesh = curved_face_mesh();
        mesh.faces.clear();
        let hit = [45f32.to_radians().cos(), 45f32.to_radians().sin(), 0.5];
        let normal = [hit[0], hit[1], 0.0];
        let sub = face_submesh(&mesh, hit, normal).unwrap();
        assert_eq!(
            sub.indices.len(),
            6,
            "the plane through the hit is one strip"
        );
    }

    #[test]
    fn no_matching_plane_returns_none() {
        let mesh = two_face_mesh();
        // Side plane: no triangles align with +X.
        assert!(coplanar_face_submesh(&mesh, [1.0, 0.5, 0.5], [1.0, 0.0, 0.0]).is_none());
    }
}
