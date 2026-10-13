//! Window-event handling and workbench input dispatch.

use core_document::{MouseButton as WbMouseButton, WorkbenchId, WorkbenchInputEvent};
use glam::{Vec2, Vec3};
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use render_wgpu::RenderBackend;
use uuid::Uuid;
use web_time::Instant;

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
            self.last_input_time = Some(web_time::Instant::now());
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
            // A release outside the window never arrives.
            WindowEvent::Focused(false) => {
                self.box_select = None;
                self.mouse_buttons_down = 0;
                self.held_buttons = [false; 3];
                self.session.camera.forget_released([false; 3]);
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

            // The box's corner follows the pointer, held to the view.
            if let Some(drag) = self.box_select.as_mut() {
                drag.to = Vec2::new(
                    cursor_x.clamp(0.0, vp.2 as f32),
                    cursor_y.clamp(0.0, vp.3 as f32),
                );
                self.redraw_needed = true;
            }

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
                // editing: the sketcher renders its own hover state.
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
                self.session.hovered_triangle = None;
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
        // The array task's spacing handle, before a bench or the camera.
        if self.array_handle_input(&event) {
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

        let orbit_pick = self.session.hovered_world_pos.map(Vec3::from_array);
        let cam_res =
            self.session
                .camera
                .on_viewport_pointer(&event, &self.user_settings.camera, orbit_pick);
        redraw |= cam_res.wants_redraw();
        if matches!(cam_res, CameraPointerResult::LmbReleasedMaybeSelect) && !boxed {
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
    fn datum_under_cursor(&self) -> Option<crate::app::measure::MeasurePick> {
        let id = self.session.hovered_feature?;
        let document = &self.session.document;
        let node = document.feature_tree().get_node(id)?;
        if node.workbench_id.as_str() != core_document::DATUM_KIND {
            return None;
        }
        let feature: core_document::DatumFeature =
            serde_json::from_value(document.feature_values(id)?.clone()).ok()?;
        let placement = node
            .body
            .map(|body| document.body_placement(body))
            .unwrap_or_default();
        let cursor = self.cursor_in_viewport?;
        let vp = self.session.camera.viewport_info();
        let (eye, ray) = core_document::runtime::viewport_to_ray(
            self.session.camera.view_projection(),
            (vp.0 as u32, vp.1 as u32, vp.2, vp.3),
            cursor,
        )?;
        let (datum, point) = crate::app::measure::DatumPick::of(
            &feature,
            placement,
            Vec3::from_array(eye),
            Vec3::from_array(ray),
        );
        Some(crate::app::measure::MeasurePick::Datum {
            name: node.name.clone(),
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
        self.session.viewport_menu = body.map(|body| crate::ui::ViewportMenu {
            body: core_document::BodyId(body),
            at: [(vp.0 + cx) / scale, (vp.1 + cy) / scale],
            face,
            point,
            repeat,
        });
        true
    }

    /// Drop what is picked in the view: the body, its faces and its edges.
    pub(crate) fn clear_view_selection(&mut self) {
        self.session.selected_body = None;
        self.session.last_face_hit = None;
        self.session.earlier_faces.clear();
        self.session.earlier_bodies.clear();
        self.session.face_highlight = None;
        self.session.selected_edges.clear();
        self.session.last_select_click = None;
    }

    pub(crate) fn toggle_body_under_cursor_selection(&mut self) -> bool {
        // A click picks one body at most; the others a box took go.
        self.session.earlier_bodies.clear();
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
            if is_double || whole_mesh || (whole_only && !already_whole) {
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
                let picked = self.selected_face_refs().len();
                if picked > 1 {
                    app_log::info(format!("Selected {picked} faces"));
                } else {
                    app_log::info(
                        "Selected face (Ctrl adds more; double-click for the whole body)",
                    );
                }
            }
        } else if self.session.selected_body.is_some() || !self.session.selected_edges.is_empty() {
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

    /// The triangle the pick drew under the cursor, when `body` is the one
    /// it found there.
    pub(crate) fn hovered_triangle_of(&self, body: Uuid) -> Option<HoveredTriangle> {
        self.session
            .hovered_triangle
            .filter(|_| self.session.hovered_body == Some(body))
    }
}

/// A triangle the pick pass drew under the cursor, as an index into the
/// mesh the document holds for the picked body at `revision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HoveredTriangle {
    pub index: u32,
    pub revision: u64,
}

/// The picked triangle as one of the document's: `drawn` is the body's
/// entry in `DocumentSession::drawn_meshes`, the revision it was last
/// submitted with and its geometry's. A pick drawn with another revision,
/// or a body drawn with a mesh of its own, names no triangle of the
/// document's.
pub(crate) fn document_triangle(
    picked: render_wgpu::PickedTriangle,
    drawn: Option<(u64, u64)>,
) -> Option<HoveredTriangle> {
    let (submitted, geometry) = drawn?;
    (submitted == picked.revision).then_some(HoveredTriangle {
        index: picked.index,
        revision: geometry,
    })
}

/// The triangle `pick` names in `mesh`, the document's mesh at `revision`:
/// its index, one vertex and unit normal. `None` when the pick was taken
/// against another revision, or names no triangle of the mesh with an
/// area.
fn picked_triangle(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
) -> Option<(usize, glam::Vec3, glam::Vec3)> {
    let pick = pick.filter(|p| p.revision == revision)?;
    let index = pick.index as usize;
    let tri = mesh.indices.as_chunks::<3>().0.get(index)?;
    let corner = |i: u32| {
        mesh.positions
            .get(i as usize)
            .map(|p| glam::Vec3::from_array(*p))
    };
    let (a, b, c) = (corner(tri[0])?, corner(tri[1])?, corner(tri[2])?);
    let normal = (b - a).cross(c - a);
    (normal.length_squared() >= 1e-12).then(|| (index, a, normal.normalize()))
}

/// The face under the cursor on `mesh`, the document's mesh at `revision`:
/// on the triangle `pick` names, else on the one nearest `point`.
pub(crate) fn face_ref_at_pick(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
    point: glam::Vec3,
) -> Option<core_document::FaceRef> {
    match picked_triangle(mesh, revision, pick) {
        Some((triangle, anchor, normal)) => {
            Some(face_ref_on(mesh, triangle, anchor, normal, point))
        }
        None => face_ref_from_mesh(mesh, point),
    }
}

/// The kernel face under the cursor on `mesh`, the document's mesh at
/// `revision`, when the mesh records faces: the face of the triangle
/// `pick` names, else of the one nearest `point`.
pub(crate) fn face_id_at_pick(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
    point: [f32; 3],
) -> Option<u32> {
    if mesh.faces.len() != mesh.indices.len() / 3 || mesh.faces.is_empty() {
        return None;
    }
    match picked_triangle(mesh, revision, pick) {
        Some((triangle, _, _)) => Some(mesh.faces[triangle]),
        None => face_id_at(mesh, point),
    }
}

/// Resolve a picked world position to a face reference on `mesh`.
///
/// The GPU pick reconstructs the position from the depth buffer, whose
/// precision varies with view angle and distance: the raw point can sit a
/// millimetre or more off the surface, far enough that a naive containment
/// test misses the face and falls back to whole-body selection. So:
/// find the nearest triangle, take the face plane from ITS exact vertices,
/// and project the picked point onto that plane.
pub(crate) fn face_ref_from_mesh(
    mesh: &kernel_api::TriMesh,
    point: glam::Vec3,
) -> Option<core_document::FaceRef> {
    let (triangle, dist_sq, anchor, normal) = nearest_triangle(mesh, point)?;

    // Sanity bound relative to the model size: the pick already identified
    // this body, so the nearest triangle is the right face unless the
    // position is wildly stale.
    let (min, max) = mesh.bounds()?;
    let diag = (glam::Vec3::from_array(max) - glam::Vec3::from_array(min)).length();
    let limit = (diag * 0.05).max(1.0);
    if dist_sq > limit * limit {
        return None;
    }
    Some(face_ref_on(mesh, triangle, anchor, normal, point))
}

/// The face of `triangle` (one vertex `anchor`, unit `normal`) at `point`.
fn face_ref_on(
    mesh: &kernel_api::TriMesh,
    triangle: usize,
    anchor: glam::Vec3,
    normal: glam::Vec3,
    point: glam::Vec3,
) -> core_document::FaceRef {
    // Project the noisy picked point onto the triangle's exact plane so the
    // face plane (and any sketch placed on it) is depth-error free.
    let projected = point - normal * (point - anchor).dot(normal);
    let surface = mesh
        .faces
        .get(triangle)
        .and_then(|face| mesh.face_surfaces.get(*face as usize))
        .copied();
    let name = mesh
        .faces
        .get(triangle)
        .and_then(|face| mesh.face_names.get(*face as usize))
        .copied()
        .unwrap_or(0);
    core_document::FaceRef {
        name,
        point: projected.to_array(),
        normal: normal.to_array(),
        surface,
    }
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
/// is the one under the hit, whole: a cylinder wall as much as a flat side.
/// A mesh without that (a sketch, a document saved before faces were
/// recorded) falls back to the plane through the hit, which is exact for a
/// flat face and one strip of a curved one. The face is the one `pick`
/// names on the document's mesh at `revision` ([`face_id_at_pick`]);
/// returned with the sub-mesh, when the mesh records faces.
pub(crate) fn face_submesh(
    mesh: &kernel_api::TriMesh,
    revision: u64,
    pick: Option<HoveredTriangle>,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<(Option<u32>, kernel_api::TriMesh)> {
    if let Some(face) = face_id_at_pick(mesh, revision, pick, point) {
        return Some((Some(face), face_submesh_by_id(mesh, face)?));
    }
    Some((None, coplanar_face_submesh(mesh, point, normal)?))
}

/// The kernel face the triangle nearest `point` was cut from, when the mesh
/// records faces.
pub(crate) fn face_id_at(mesh: &kernel_api::TriMesh, point: [f32; 3]) -> Option<u32> {
    if mesh.faces.len() != mesh.indices.len() / 3 || mesh.faces.is_empty() {
        return None;
    }
    let (tri, _, _, _) = nearest_triangle(mesh, glam::Vec3::from_array(point))?;
    Some(mesh.faces[tri])
}

/// The face under the cursor, drawn translucent in the hover paint: the
/// body the pick found, the kernel face at the picked point, and the copy
/// of that face. Rebuilt only when the picked point moves.
pub(crate) struct FaceHover {
    pub body: Uuid,
    pub face: u32,
    pub revision: u64,
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// The picked point this hover was resolved for.
    pub probe: [f32; 3],
}

/// Where face `face` of `mesh` stands and the way it faces: the centre of
/// its area and its area-weighted outward normal, from the mesh's own
/// normals (the kernel's, outward, where the winding may not be).
pub(crate) fn face_frame(
    mesh: &kernel_api::TriMesh,
    face: u32,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let mut area_sum = 0.0;
    let mut centre = glam::Vec3::ZERO;
    let mut normal = glam::Vec3::ZERO;
    for (tri, id) in mesh.indices.as_chunks::<3>().0.iter().zip(&mesh.faces) {
        if *id != face {
            continue;
        }
        let p = |i: u32| {
            mesh.positions
                .get(i as usize)
                .map(|p| glam::Vec3::from_array(*p))
        };
        let (a, b, c) = (p(tri[0])?, p(tri[1])?, p(tri[2])?);
        let area = (b - a).cross(c - a).length() / 2.0;
        area_sum += area;
        centre += (a + b + c) / 3.0 * area;
        for i in tri {
            if let Some(n) = mesh.normals.get(*i as usize) {
                normal += glam::Vec3::from_array(*n) * area;
            }
        }
    }
    (area_sum > 0.0 && normal.length_squared() > 1e-12)
        .then(|| (centre / area_sum, normal.normalize()))
}

/// Every triangle cut from kernel face `face`, copied exactly: the
/// translucent pass passes equal depth, so the copy sits on its surface
/// whichever way the face is wound. A copy lifted along the winding normal
/// would sink into the solid on a face wound inward and never show.
pub(crate) fn face_submesh_by_id(
    mesh: &kernel_api::TriMesh,
    face: u32,
) -> Option<kernel_api::TriMesh> {
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
            out.positions.push(v.to_array());
            out.normals.push(n.to_array());
        }
        out.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    (!out.indices.is_empty()).then_some(out)
}

/// The faces picked, in order, after Ctrl picks `face`: taken out when it
/// is among them (by its kernel face), added at the end otherwise.
pub(crate) fn toggled(mut picked: Vec<PickedFace>, face: PickedFace) -> Vec<PickedFace> {
    let id = face.highlight.face;
    match picked.iter().position(|p| {
        id.is_some() && p.highlight.body == face.highlight.body && p.highlight.face == id
    }) {
        Some(i) => {
            picked.remove(i);
        }
        None => picked.push(face),
    }
    picked
}

/// A face picked before the last, kept with its highlight.
#[derive(Debug, Clone)]
pub(crate) struct PickedFace {
    pub face: core_document::FaceRef,
    pub highlight: FaceHighlight,
}

/// Sub-mesh rendered as a picked face's highlight.
#[derive(Debug, Clone)]
pub(crate) struct FaceHighlight {
    pub body: Uuid,
    /// The kernel face, when the mesh records faces.
    pub face: Option<u32>,
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// Bumped whenever the sub-mesh is re-extracted.
    pub revision: u64,
}

/// Every triangle of `mesh` lying on the plane (point, normal), copied
/// exactly. The face as geometry sees it: exact for a flat face, one strip
/// of a curved one, and two flat faces on one plane together.
pub(crate) fn coplanar_face_submesh(
    mesh: &kernel_api::TriMesh,
    point: [f32; 3],
    normal: [f32; 3],
) -> Option<kernel_api::TriMesh> {
    const NORMAL_ALIGN: f32 = 0.999;
    const PLANE_TOL: f32 = 0.05;
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
            positions.push(v.to_array());
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
        ..kernel_api::TriMesh::default()
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
mod tests {

    /// A face's frame is the centre of its area and the way it faces out,
    /// read from the mesh's normals whatever the triangles' winding.
    #[test]
    fn a_face_frame_is_its_centre_and_outward_normal() {
        // A 2 × 2 square at z = 3 facing +z, one triangle wound backwards,
        // beside a second face.
        let mesh = kernel_api::TriMesh {
            positions: vec![
                [0.0, 0.0, 3.0],
                [2.0, 0.0, 3.0],
                [2.0, 2.0, 3.0],
                [0.0, 2.0, 3.0],
                [9.0, 9.0, 9.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 5],
            indices: vec![0, 1, 2, 0, 3, 2, 1, 2, 4],
            faces: vec![7, 7, 8],
            ..Default::default()
        };
        let (centre, normal) = super::face_frame(&mesh, 7).expect("face 7 is there");
        assert!(
            (centre - glam::Vec3::new(1.0, 1.0, 3.0)).length() < 1e-5,
            "{centre}"
        );
        assert!((normal - glam::Vec3::Z).length() < 1e-5, "{normal}");
        assert!(super::face_frame(&mesh, 3).is_none());
    }

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
            ..TriMesh::default()
        }
    }

    #[test]
    fn extracts_only_the_hit_plane_exactly_where_it_lies() {
        let mesh = two_face_mesh();
        let sub = coplanar_face_submesh(&mesh, [0.5, 0.5, 1.0], [0.0, 0.0, 1.0]).unwrap();
        assert_eq!(sub.indices.len(), 6, "only the top quad's two triangles");
        assert!(
            sub.positions.iter().all(|p| (p[2] - 1.0).abs() < 1e-6),
            "the copy sits on the face, not above it"
        );
        // No triangle lies in a side plane.
        assert!(coplanar_face_submesh(&mesh, [1.0, 0.5, 0.5], [1.0, 0.0, 0.0]).is_none());
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
        let sub = coplanar_face_submesh(&mesh, face.point, face.normal).unwrap();
        assert_eq!(sub.indices.len(), 6);
        assert!(
            face_ref_from_mesh(&mesh, glam::Vec3::new(50.0, 50.0, 50.0)).is_none(),
            "a point far off every face is no pick"
        );
    }

    /// A point between the two faces of [`two_face_mesh`], nearer the
    /// bottom: where depth noise could put a pick on the top face of a thin
    /// wall.
    const BETWEEN: glam::Vec3 = glam::Vec3::new(0.5, 0.5, 0.3);

    fn top_and_bottom() -> TriMesh {
        TriMesh {
            faces: vec![1, 1, 2, 2],
            ..two_face_mesh()
        }
    }

    #[test]
    fn a_picked_triangle_names_its_face_wherever_the_point_lies() {
        use super::{HoveredTriangle, face_id_at_pick, face_ref_at_pick};
        let mesh = top_and_bottom();
        let top = Some(HoveredTriangle {
            index: 1,
            revision: 5,
        });
        let face = face_ref_at_pick(&mesh, 5, top, BETWEEN).unwrap();
        assert!((glam::Vec3::from_array(face.normal) - glam::Vec3::Z).length() < 1e-4);
        assert!(
            (face.point[2] - 1.0).abs() < 1e-6,
            "projected onto the picked triangle's plane: {:?}",
            face.point
        );
        assert_eq!(face_id_at_pick(&mesh, 5, top, BETWEEN.to_array()), Some(1));
        let (id, sub) =
            super::face_submesh(&mesh, 5, top, BETWEEN.to_array(), face.normal).unwrap();
        assert_eq!(id, Some(1));
        assert!(
            sub.positions.iter().all(|p| p[2] == 1.0),
            "the highlight is the top face"
        );
    }

    #[test]
    fn a_stale_or_foreign_triangle_falls_back_to_the_nearest() {
        use super::{HoveredTriangle, face_id_at_pick, face_ref_at_pick};
        let mesh = top_and_bottom();
        let picks = [
            // Taken against another revision of the mesh.
            Some(HoveredTriangle {
                index: 1,
                revision: 4,
            }),
            // Past the mesh's triangles.
            Some(HoveredTriangle {
                index: 4,
                revision: 5,
            }),
            None,
        ];
        for pick in picks {
            let face = face_ref_at_pick(&mesh, 5, pick, BETWEEN).unwrap();
            assert!(face.point[2].abs() < 1e-6, "{pick:?}: the bottom, nearest");
            assert_eq!(
                face_id_at_pick(&mesh, 5, pick, BETWEEN.to_array()),
                Some(2),
                "{pick:?}"
            );
        }
    }

    #[test]
    fn a_pick_counts_in_the_document_mesh_only_as_it_was_drawn() {
        use super::{HoveredTriangle, document_triangle};
        let picked = render_wgpu::PickedTriangle {
            index: 3,
            revision: 9,
        };
        assert_eq!(
            document_triangle(picked, Some((9, 2))),
            Some(HoveredTriangle {
                index: 3,
                revision: 2
            }),
            "drawn as submitted: the geometry's revision"
        );
        assert_eq!(
            document_triangle(picked, Some((10, 2))),
            None,
            "re-meshed since"
        );
        assert_eq!(
            document_triangle(picked, None),
            None,
            "drawn with its own mesh"
        );
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

    /// A hit on the middle strip selects all of its kernel face; with no
    /// face ids, only the plane through the hit.
    #[test]
    fn a_curved_face_selects_whole_when_the_mesh_knows_its_faces() {
        use super::face_submesh;
        let mut mesh = curved_face_mesh();
        let hit = [45f32.to_radians().cos(), 45f32.to_radians().sin(), 0.5];
        let normal = [hit[0], hit[1], 0.0];
        let (face, sub) = face_submesh(&mesh, 0, None, hit, normal).unwrap();
        assert_eq!(face, Some(7));
        assert_eq!(
            sub.indices.len(),
            3 * 6,
            "all three strips of face 7, not the fourth"
        );
        mesh.faces.clear();
        let (face, sub) = face_submesh(&mesh, 0, None, hit, normal).unwrap();
        assert_eq!(face, None);
        assert_eq!(
            sub.indices.len(),
            6,
            "the plane through the hit is one strip"
        );
    }
}

#[cfg(test)]
mod picking_faces {
    use super::*;

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
