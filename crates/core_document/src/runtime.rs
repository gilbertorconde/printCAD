//! Runtime context and hooks for workbenches.
//!
//! This module provides the runtime API that workbenches use to interact with
//! the application shell: logging, document access, camera/picking info, and
//! overlay drawing.

use crate::{Document, FeatureId};

/// Log levels for workbench messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

/// A pending log entry from a workbench.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
}

/// Runtime context passed to workbench hooks.
///
/// This is the primary interface workbenches use to interact with the host
/// application. It provides:
/// - Logging (routed to the in-app log panel)
/// - Read/write access to the active document
/// - Camera and viewport information (read-only)
/// - Picking/selection state
/// - Overlay drawing registration (for tool visualizations)
pub struct WorkbenchRuntimeContext<'a> {
    /// The active document (mutable access for edits).
    pub document: &'a mut Document,

    /// Pending log entries to be flushed by the host after the hook returns.
    pending_logs: Vec<LogEntry>,

    /// Current camera position in world space.
    pub camera_position: [f32; 3],

    /// Current camera target (orbit center) in world space.
    pub camera_target: [f32; 3],

    /// Viewport dimensions (x, y, width, height) in pixels.
    pub viewport: (u32, u32, u32, u32),

    /// View-projection matrix for transforming 3D world coordinates to clip space.
    /// Used for projecting 3D points to screen coordinates.
    pub view_proj: Option<[[f32; 4]; 4]>,

    /// World position under the cursor (if any geometry is hovered).
    pub hovered_world_pos: Option<[f32; 3]>,

    /// ID of the body currently under the cursor (if any).
    pub hovered_body_id: Option<uuid::Uuid>,

    /// ID of the currently selected body (if any).
    pub selected_body_id: Option<uuid::Uuid>,

    /// Active document object (selected feature in tree - separate from editing mode).
    pub active_document_object: Option<FeatureId>,

    /// Current cursor position in viewport-local coordinates (if inside viewport).
    pub cursor_viewport_pos: Option<(f32, f32)>,

    /// Request camera orientation to a plane (set by workbench, read by host).
    pub camera_orient_request: Option<CameraOrientRequest>,

    /// Request to exit sketch mode (set by workbench UI, read by host).
    pub finish_sketch_requested: bool,

    /// Workbench → host: switch the active workbench after this hook
    /// returns (e.g. Part Design's "New Sketch" jumps to the sketcher).
    pub workbench_switch_request: Option<crate::WorkbenchId>,

    /// Host ⇄ sketch workbench: a pending "create a sketch on this body"
    /// request. Set by a requesting workbench, carried by the host between
    /// hooks, and TAKEN by the sketch workbench when it starts its plane
    /// picker.
    pub start_sketch_on_body: Option<SketchAttachRequest>,

    /// Host → workbench: whether Ctrl is held (multi-select modifier).
    pub ctrl_down: bool,

    /// Host → workbench: the face under the last body selection, when the
    /// GPU pick landed on solid geometry (surface point + outward normal in
    /// world space). Lets "New Sketch" attach to the clicked face.
    pub selected_face: Option<FaceRef>,
}

/// A picked face on a solid body: a point on the surface and its outward
/// normal, both in world space (millimetres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceRef {
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// Request to create a sketch attached to a body, optionally referenced on
/// one of its faces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SketchAttachRequest {
    pub body: uuid::Uuid,
    pub face: Option<FaceRef>,
}

/// Request to orient camera to a specific plane.
#[derive(Debug, Clone)]
pub struct CameraOrientRequest {
    pub plane_origin: [f32; 3],
    pub plane_normal: [f32; 3],
    pub plane_up: [f32; 3],
}

impl<'a> WorkbenchRuntimeContext<'a> {
    /// Create a new runtime context.
    pub fn new(
        document: &'a mut Document,
        camera_position: [f32; 3],
        camera_target: [f32; 3],
        viewport: (u32, u32, u32, u32),
    ) -> Self {
        Self {
            document,
            pending_logs: Vec::new(),
            camera_position,
            camera_target,
            viewport,
            hovered_world_pos: None,
            hovered_body_id: None,
            selected_body_id: None,
            cursor_viewport_pos: None,
            camera_orient_request: None,
            finish_sketch_requested: false,
            active_document_object: None,
            view_proj: None,
            workbench_switch_request: None,
            start_sketch_on_body: None,
            selected_face: None,
            ctrl_down: false,
        }
    }

    /// Log an info message to the application log panel.
    pub fn log_info(&mut self, message: impl Into<String>) {
        self.pending_logs.push(LogEntry {
            level: LogLevel::Info,
            message: message.into(),
        });
    }

    /// Log a warning message to the application log panel.
    pub fn log_warn(&mut self, message: impl Into<String>) {
        self.pending_logs.push(LogEntry {
            level: LogLevel::Warn,
            message: message.into(),
        });
    }

    /// Log an error message to the application log panel.
    pub fn log_error(&mut self, message: impl Into<String>) {
        self.pending_logs.push(LogEntry {
            level: LogLevel::Error,
            message: message.into(),
        });
    }

    /// Drain pending log entries (called by host after hook returns).
    pub fn drain_logs(&mut self) -> Vec<LogEntry> {
        std::mem::take(&mut self.pending_logs)
    }

    /// Convert a world position to viewport-local pixel coordinates (the
    /// same space as [`Self::cursor_viewport_pos`], i.e. without the
    /// viewport's screen offset).
    ///
    /// NDC is Y-down: the host camera bakes the Vulkan Y flip into
    /// `view_proj`, so no flip is applied here (this mirrors the app shell's
    /// `CameraController::world_to_screen`).
    ///
    /// Returns `None` when no `view_proj` was provided, the viewport is
    /// degenerate, or the point is behind the camera.
    pub fn world_to_viewport(&self, world_pos: [f32; 3]) -> Option<(f32, f32)> {
        let view_proj = glam::Mat4::from_cols_array_2d(&self.view_proj?);
        let (_, _, w, h) = self.viewport;
        if w == 0 || h == 0 {
            return None;
        }
        let clip = view_proj * glam::Vec3::from_array(world_pos).extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some((
            (ndc.x + 1.0) * 0.5 * w as f32,
            (ndc.y + 1.0) * 0.5 * h as f32,
        ))
    }

    /// Convert viewport-local pixel coordinates to a world-space ray.
    /// Returns `(origin, direction)` with `direction` normalized, or `None`
    /// when no `view_proj` was provided or the unprojection is degenerate.
    ///
    /// Depth follows the Vulkan convention (near plane at NDC z = 0, far at
    /// z = 1); the ray origin sits on the near plane.
    pub fn viewport_to_ray(&self, viewport_pos: (f32, f32)) -> Option<([f32; 3], [f32; 3])> {
        let view_proj = glam::Mat4::from_cols_array_2d(&self.view_proj?);
        let (_, _, w, h) = self.viewport;
        if w == 0 || h == 0 {
            return None;
        }
        let ndc_x = (viewport_pos.0 / w as f32) * 2.0 - 1.0;
        let ndc_y = (viewport_pos.1 / h as f32) * 2.0 - 1.0;
        let inv = view_proj.inverse();
        let near_clip = inv * glam::Vec4::new(ndc_x, ndc_y, 0.0, 1.0);
        let far_clip = inv * glam::Vec4::new(ndc_x, ndc_y, 1.0, 1.0);
        if near_clip.w.abs() < 1e-12 || far_clip.w.abs() < 1e-12 {
            return None;
        }
        let near_world = near_clip.truncate() / near_clip.w;
        let far_world = far_clip.truncate() / far_clip.w;
        let dir = far_world - near_world;
        if dir.length_squared() < 1e-24 {
            return None;
        }
        Some((near_world.to_array(), dir.normalize().to_array()))
    }

    /// Convert viewport-local pixel coordinates to the intersection of the
    /// cursor ray with a plane. Returns `None` when the ray is (nearly)
    /// parallel to the plane or the intersection lies behind the near plane.
    pub fn viewport_to_plane(
        &self,
        viewport_pos: (f32, f32),
        plane_origin: [f32; 3],
        plane_normal: [f32; 3],
    ) -> Option<[f32; 3]> {
        let (origin, dir) = self.viewport_to_ray(viewport_pos)?;
        let origin = glam::Vec3::from_array(origin);
        let dir = glam::Vec3::from_array(dir);
        let normal = glam::Vec3::from_array(plane_normal).normalize();
        let denom = dir.dot(normal);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (glam::Vec3::from_array(plane_origin) - origin).dot(normal) / denom;
        if t < 0.0 {
            return None;
        }
        Some((origin + dir * t).to_array())
    }
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    use glam::{Mat4, Vec3, Vec4};

    /// Build a Vulkan-convention view-projection like the app camera does:
    /// perspective (0..1 depth) with the Y flip baked in, looking at the
    /// origin from +Z.
    fn test_ctx_matrix() -> [[f32; 4]; 4] {
        let proj = Mat4::perspective_rh(60f32.to_radians(), 4.0 / 3.0, 0.1, 100.0);
        let flip_y = Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0));
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 10.0), Vec3::ZERO, Vec3::Y);
        (flip_y * proj * view).to_cols_array_2d()
    }

    fn ctx_with_matrix(document: &mut Document) -> WorkbenchRuntimeContext<'_> {
        let mut ctx = WorkbenchRuntimeContext::new(
            document,
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 0.0],
            (0, 0, 800, 600),
        );
        ctx.view_proj = Some(test_ctx_matrix());
        ctx
    }

    #[test]
    fn center_of_viewport_projects_to_camera_target() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        let (x, y) = ctx.world_to_viewport([0.0, 0.0, 0.0]).unwrap();
        assert!((x - 400.0).abs() < 0.5, "x = {x}");
        assert!((y - 300.0).abs() < 0.5, "y = {y}");
    }

    #[test]
    fn behind_camera_returns_none() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        assert!(ctx.world_to_viewport([0.0, 0.0, 20.0]).is_none());
    }

    #[test]
    fn no_view_proj_returns_none() {
        let mut doc = Document::new("t");
        let ctx = WorkbenchRuntimeContext::new(
            &mut doc,
            [0.0, 0.0, 10.0],
            [0.0, 0.0, 0.0],
            (0, 0, 800, 600),
        );
        assert!(ctx.world_to_viewport([0.0, 0.0, 0.0]).is_none());
        assert!(ctx.viewport_to_ray((400.0, 300.0)).is_none());
    }

    #[test]
    fn center_ray_passes_through_target() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        let (origin, dir) = ctx.viewport_to_ray((400.0, 300.0)).unwrap();
        let origin = Vec3::from_array(origin);
        let dir = Vec3::from_array(dir);
        // Ray from the camera through the viewport center should pass
        // (very close to) the world origin.
        let t = (Vec3::ZERO - origin).dot(dir);
        let closest = origin + dir * t;
        assert!(closest.length() < 1e-3, "closest = {closest}");
        // And it points away from the camera (-Z).
        assert!(dir.z < 0.0);
    }

    #[test]
    fn plane_hit_roundtrips_through_projection() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        // Pick an arbitrary viewport point, intersect the Z=0 plane, and
        // project the hit back: it must land on the original pixel.
        let px = (513.0, 222.0);
        let hit = ctx
            .viewport_to_plane(px, [0.0, 0.0, 0.0], [0.0, 0.0, 1.0])
            .unwrap();
        assert!(hit[2].abs() < 1e-4);
        let (x, y) = ctx.world_to_viewport(hit).unwrap();
        assert!((x - px.0).abs() < 0.05, "x = {x}");
        assert!((y - px.1).abs() < 0.05, "y = {y}");
    }

    #[test]
    fn parallel_plane_returns_none() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        // The center ray travels along -Z; a plane containing that axis
        // (normal +Y at the ray height) is parallel to it.
        assert!(ctx
            .viewport_to_plane((400.0, 300.0), [0.0, 5.0, 0.0], [0.0, 1.0, 0.0])
            .is_none());
    }

    #[test]
    fn y_axis_is_screen_down() {
        let mut doc = Document::new("t");
        let ctx = ctx_with_matrix(&mut doc);
        // World +Y should appear ABOVE the center on screen, i.e. smaller
        // viewport y (Y-down pixels, flip baked into the matrix).
        let (_, y_up) = ctx.world_to_viewport([0.0, 2.0, 0.0]).unwrap();
        let (_, y_center) = ctx.world_to_viewport([0.0, 0.0, 0.0]).unwrap();
        assert!(y_up < y_center, "y_up = {y_up}, y_center = {y_center}");
    }

    #[test]
    fn used_vec4_sanity() {
        // Guard against accidental column/row-major confusion in
        // from_cols_array_2d usage: transform a known point both ways.
        let m = Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0));
        let arr = m.to_cols_array_2d();
        let back = Mat4::from_cols_array_2d(&arr);
        assert_eq!(
            back * Vec4::new(0.0, 0.0, 0.0, 1.0),
            Vec4::new(1.0, 2.0, 3.0, 1.0)
        );
    }
}

/// Input event passed to workbench on_input hook.
#[derive(Debug, Clone)]
pub enum WorkbenchInputEvent {
    /// Mouse button pressed.
    MousePress {
        button: MouseButton,
        viewport_pos: (f32, f32),
    },
    /// Mouse button released.
    MouseRelease {
        button: MouseButton,
        viewport_pos: (f32, f32),
    },
    /// Mouse moved.
    MouseMove { viewport_pos: (f32, f32) },
    /// Key pressed.
    KeyPress { key: KeyCode },
    /// Key released.
    KeyRelease { key: KeyCode },
}

/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Other(u16),
}

/// Simplified key code (extend as needed).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Escape,
    Enter,
    Space,
    Delete,
    Backspace,
    Tab,
    // Letters
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    // Numbers
    Key0,
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,
    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    // Modifiers (for reference; actual modifier state tracked separately)
    Shift,
    Control,
    Alt,
    // Other
    Unknown,
}

/// Result of a workbench input handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InputResult {
    /// If true, the event was consumed and should not propagate further.
    pub consumed: bool,
    /// If true, the viewport should be redrawn.
    pub redraw: bool,
}

impl InputResult {
    pub fn consumed() -> Self {
        Self {
            consumed: true,
            redraw: true,
        }
    }

    pub fn ignored() -> Self {
        Self::default()
    }

    pub fn redraw_only() -> Self {
        Self {
            consumed: false,
            redraw: true,
        }
    }
}
