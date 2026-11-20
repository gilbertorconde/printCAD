//! A bench's tools, the input it takes and what it draws in the view.

use super::*;

/// What a bench does while it is active: its tools' buttons, the input
/// it takes, and what it shows over and in the viewport.
pub trait BenchTools {
    /// While this bench has an edit session open (`editing_feature` is
    /// `Some`) the view stays square to its plane: orbit off, pan, zoom
    /// and roll on.
    fn locks_view_to_plane(&self) -> bool {
        false
    }

    /// Whether the origin's planes draw now, whatever the view's setting
    /// says, for the user to pick one: the plane under the cursor reaches
    /// the bench as `ctx.hovered_base_plane`.
    fn shows_origin_planes(&self) -> bool {
        false
    }

    /// Whether the bench is taking a typed number from the keyboard right
    /// now (a length while drawing). Bare digits, `.`, `,` and `-` stay
    /// with it rather than running their shortcuts.
    fn takes_numeric_input(&self) -> bool {
        false
    }

    /// Whether the bench is in the middle of a typed value (a formula):
    /// every key that types text without Ctrl or Alt stays with it, letters
    /// included, rather than running its shortcut.
    fn takes_text_input(&self) -> bool {
        false
    }

    /// The host's answer to this bench's
    /// [`HostRequest::CloseToolGesture`](crate::HostRequest::CloseToolGesture)
    /// or [`HostRequest::UndoToolGesture`](crate::HostRequest::UndoToolGesture),
    /// right after the hook that asked.
    fn tool_history(
        &mut self,
        _ctx: &mut WorkbenchRuntimeContext,
        _reply: crate::runtime::ToolHistoryReply,
    ) {
    }

    /// Whether the bench fills a strip under the view while active, as it
    /// last drew; its drawing side draws it.
    fn has_viewport_strip(&self) -> bool {
        false
    }

    /// Called when an input event occurs while this workbench is active.
    /// Return `InputResult::consumed()` to prevent further event propagation.
    fn on_input(
        &mut self,
        _event: &WorkbenchInputEvent,
        _active_tool: Option<&str>,
        _ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        InputResult::ignored()
    }

    /// Cancel a held pointer gesture when the window loses focus. A release
    /// outside the window may not arrive; restore the gesture's press state.
    fn cancel_pointer_gesture(&mut self, _ctx: &mut WorkbenchRuntimeContext) {}

    /// What the bench shows over and in the viewport this frame, asked
    /// once a frame under one context. Read-only; a bench fills what it
    /// draws and leaves the rest at its default.
    fn viewport(&self, _ctx: &WorkbenchRuntimeContext) -> ViewportOutput {
        ViewportOutput::default()
    }

    /// Whether an Action tool that acts as a toggle is currently on, so its
    /// button can render pressed.
    fn tool_toggled(&self, _tool_id: &str) -> bool {
        false
    }

    /// Whether a tool's button is enabled given the current runtime
    /// context; every tool is by default.
    fn is_tool_enabled(&self, _tool_id: &str, _ctx: &WorkbenchRuntimeContext) -> bool {
        true
    }
}
