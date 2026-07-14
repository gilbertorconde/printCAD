//! Window + GPU resource bundle with a structurally enforced teardown order.

use crate::ui::UiLayer;
use render_vk::VulkanRenderer;
use winit::window::{Window, WindowId};

/// Everything that only exists while the native window is alive.
///
/// Field order IS the teardown contract: struct fields drop in *declaration*
/// order, so `renderer` tears down its surface/swapchain while `window` is
/// still alive. Do not reorder the fields.
pub(crate) struct Gfx {
    /// Drops first: GPU teardown needs the native window to still exist.
    pub renderer: VulkanRenderer,
    pub ui_layer: UiLayer,
    /// Drops last.
    pub window: Window,
    pub window_id: WindowId,
}
