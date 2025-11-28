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

    /// Convert a world position to viewport coordinates.
    /// Returns None if the point is behind the camera or outside the viewport.
    /// (Stub: actual implementation requires view-projection matrix from host.)
    pub fn world_to_viewport(&self, _world_pos: [f32; 3]) -> Option<(f32, f32)> {
        // TODO: Host should provide the actual transform
        None
    }

    /// Convert viewport coordinates to a ray in world space.
    /// Returns (origin, direction) of the ray.
    /// (Stub: actual implementation requires inverse view-projection from host.)
    pub fn viewport_to_ray(&self, _viewport_pos: (f32, f32)) -> ([f32; 3], [f32; 3]) {
        // TODO: Host should provide the actual transform
        let origin = self.camera_position;
        let direction = [0.0, 0.0, -1.0];
        (origin, direction)
    }

    /// Convert viewport coordinates to a point on a plane in world space.
    /// Returns the intersection point of the camera ray with the plane.
    /// (Stub: actual implementation requires inverse view-projection from host.)
    pub fn viewport_to_plane(
        &self,
        _viewport_pos: (f32, f32),
        _plane_origin: [f32; 3],
        _plane_normal: [f32; 3],
    ) -> Option<[f32; 3]> {
        // TODO: Host should provide the actual transform
        // For now, return None - host will need to implement this
        None
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
