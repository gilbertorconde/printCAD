//! Window-event handling and workbench input dispatch.

use core_document::{MouseButton as WbMouseButton, WorkbenchId, WorkbenchInputEvent};
use glam::{Vec2, Vec3};
use winit::{
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

use core_document::WorkbenchFeature;
use render_vk::RenderBackend;
use std::time::Instant;
use uuid::Uuid;

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
                // Sketch hover feedback (CPU hit-test; the GPU pick can't
                // reliably hit hairline sketch curves). Skipped while
                // editing — the sketcher renders its own hover state.
                self.hovered_sketch = if self.sketch_editing_active() {
                    None
                } else {
                    self.sketch_feature_under_cursor()
                };
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

        // Action-behaviour tools fire once: consume them as soon as the
        // workbench handled an event with them active.
        if let Some(tool_id) = active_tool_id {
            if result.consumed && self.tool_is_action(&wb_id, &tool_id) {
                self.active_tool.active_ids.remove(&tool_id);
            }
        }

        result
    }

    fn tool_is_action(&self, wb_id: &WorkbenchId, tool_id: &str) -> bool {
        self.registry
            .tools_for(wb_id)
            .map(|tools| {
                tools
                    .iter()
                    .any(|t| t.id == tool_id && t.behavior == core_document::ToolBehavior::Action)
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
        // Sketch curves first: their tessellated lines are far too thin for
        // the 1-pixel GPU pick to hit reliably, so clicks are matched
        // against sketch geometry on the CPU with a proper pixel tolerance.
        if let Some(feature_id) = self.sketch_feature_under_cursor() {
            self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature_id));
            self.last_face_hit = None;
            self.face_highlight = None;
            app_log::info(format!("Selected sketch {feature_id:?}"));
            return true;
        }

        if let Some(hovered) = self.hovered_body {
            // Sketch feature meshes are occasionally GPU-picked too (e.g.
            // clicking exactly on a line): same selection path.
            let feature_id = core_document::FeatureId(hovered);
            if self
                .document
                .get_feature_meta(feature_id)
                .map(|n| n.workbench_id.as_str() == "wb.sketch")
                .unwrap_or(false)
            {
                self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature_id));
                self.last_face_hit = None;
                return true;
            }

            // FreeCAD-style: the first click selects the FACE under the
            // cursor; a double click promotes to the whole body.
            let now = Instant::now();
            let is_double = self
                .last_select_click
                .map(|(t, target)| target == hovered && now.duration_since(t).as_millis() < 400)
                .unwrap_or(false);
            self.last_select_click = Some((now, hovered));

            if is_double {
                self.face_highlight = None;
                self.selected_body = Some(hovered);
                app_log::info(format!("Selected body: {hovered:?}"));
            } else if self.selected_body == Some(hovered) && self.face_highlight.is_none() {
                // Clicking an already fully-selected body deselects it.
                self.selected_body = None;
                self.last_face_hit = None;
                app_log::info("Deselected body");
            } else {
                self.selected_body = Some(hovered);
                self.last_face_hit = self
                    .face_hit_under_cursor(hovered)
                    .map(|face| (hovered, face));
                self.face_highlight = self.last_face_hit.and_then(|(body, face)| {
                    let geometry = self
                        .document
                        .imported_geometry(core_document::BodyId(body))?;
                    let submesh = coplanar_face_submesh(&geometry.mesh, face.point, face.normal)?;
                    let revision = self
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
        } else if self.selected_body.is_some() {
            self.selected_body = None;
            self.last_face_hit = None;
            self.face_highlight = None;
            self.last_select_click = None;
            app_log::info("Deselected (clicked empty space)");
        }
        true
    }

    /// Find a visible sketch whose curves pass within a few pixels of the
    /// cursor: unproject the click onto each sketch's plane and hit-test in
    /// sketch coordinates (the same math the sketcher itself uses). Hidden
    /// (pad-consumed) sketches are skipped. Returns the closest hit in
    /// pixel distance.
    fn sketch_feature_under_cursor(&self) -> Option<core_document::FeatureId> {
        const TOLERANCE_PX: f32 = 8.0;
        let cursor = self.cursor_in_viewport?;
        let view_proj = self.camera.view_projection();
        let vp = self.camera.viewport_info();
        let viewport = (vp.0 as u32, vp.1 as u32, vp.2, vp.3);

        let mut best: Option<(core_document::FeatureId, f32)> = None;
        for (id, node) in self.document.feature_tree().all_nodes() {
            if node.workbench_id.as_str() != "wb.sketch" || !node.visible {
                continue;
            }
            let Ok(feature) = wb_sketch::SketchFeature::from_json(&node.data) else {
                continue;
            };
            let plane = feature.plane;
            let Some(world) = core_document::runtime::viewport_to_plane(
                view_proj,
                viewport,
                cursor,
                plane.origin,
                plane.normal,
            ) else {
                continue;
            };
            // World hit → sketch 2D coordinates.
            let origin = glam::Vec3::from_array(plane.origin);
            let rel = glam::Vec3::from_array(world) - origin;
            let pos = wb_sketch::sketch::Vec2D::new(
                rel.dot(glam::Vec3::from_array(plane.x_axis)),
                rel.dot(glam::Vec3::from_array(plane.y_axis)),
            );
            // Pixel scale at this sketch's origin (zoom-independent
            // tolerance).
            let to_px =
                |p: [f32; 3]| core_document::runtime::world_to_viewport(view_proj, viewport, p);
            let (Some(o_px), Some(x_px)) = (
                to_px(plane.origin),
                to_px((origin + glam::Vec3::from_array(plane.x_axis)).to_array()),
            ) else {
                continue;
            };
            let px_per_unit = ((x_px.0 - o_px.0).powi(2) + (x_px.1 - o_px.1).powi(2)).sqrt();
            if px_per_unit < 1e-6 {
                continue;
            }

            if let Some(dist_units) = wb_sketch::snap::nearest_curve_distance(&feature.sketch, pos)
            {
                let dist_px = dist_units * px_per_unit;
                if dist_px <= TOLERANCE_PX && best.map(|(_, d)| dist_px < d).unwrap_or(true) {
                    best = Some((*id, dist_px));
                }
            }
        }
        best.map(|(id, _)| id)
    }

    /// Derive the face (surface point + normal) under the cursor from the
    /// picked body's mesh. Runs only on selection clicks, so a linear scan
    /// is fine.
    fn face_hit_under_cursor(&self, body: Uuid) -> Option<core_document::FaceRef> {
        let point = glam::Vec3::from_array(self.hovered_world_pos?);
        let geometry = self
            .document
            .imported_geometry(core_document::BodyId(body))?;
        face_ref_from_mesh(&geometry.mesh, point)
    }
}

/// Resolve a picked world position to a face reference on `mesh`.
///
/// The GPU pick reconstructs the position from the depth buffer, whose
/// precision varies with view angle and distance — the raw point can sit a
/// millimetre or more off the surface (which used to make face selection
/// fail on some faces and silently fall back to whole-body selection). So:
/// find the nearest triangle, take the face plane from ITS exact vertices,
/// and project the picked point onto that plane.
pub(crate) fn face_ref_from_mesh(
    mesh: &kernel_api::TriMesh,
    point: glam::Vec3,
) -> Option<core_document::FaceRef> {
    let mut best: Option<(f32, glam::Vec3, glam::Vec3)> = None; // (dist2, anchor, normal)
    for tri in mesh.indices.chunks_exact(3) {
        let a = glam::Vec3::from_array(*mesh.positions.get(tri[0] as usize)?);
        let b = glam::Vec3::from_array(*mesh.positions.get(tri[1] as usize)?);
        let c = glam::Vec3::from_array(*mesh.positions.get(tri[2] as usize)?);
        let normal = (b - a).cross(c - a);
        if normal.length_squared() < 1e-12 {
            continue;
        }
        let d = point_triangle_distance_sq(point, a, b, c);
        if best.map(|(bd, _, _)| d < bd).unwrap_or(true) {
            best = Some((d, a, normal.normalize()));
        }
    }
    let (dist_sq, anchor, normal) = best?;

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

/// Coplanar-region sub-mesh extracted around a face hit, used to render a
/// single-face selection highlight. Approximation: coplanar-but-disjoint
/// regions of the same solid highlight together (no OCCT face ids in the
/// mesh yet).
pub(crate) struct FaceHighlight {
    pub body: Uuid,
    pub mesh: std::sync::Arc<kernel_api::TriMesh>,
    /// Bumped whenever the sub-mesh is re-extracted.
    pub revision: u64,
}

/// Extract every triangle of `mesh` lying on the plane (point, normal),
/// offset slightly along the normal so the highlight never z-fights the
/// face it covers.
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
    for tri in mesh.indices.chunks_exact(3) {
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

    #[test]
    fn no_matching_plane_returns_none() {
        let mesh = two_face_mesh();
        // Side plane: no triangles align with +X.
        assert!(coplanar_face_submesh(&mesh, [1.0, 0.5, 0.5], [1.0, 0.0, 0.0]).is_none());
    }
}
