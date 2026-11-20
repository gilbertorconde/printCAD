//! What the host and the UI exchange each frame, owned by neither: the
//! state both read ([`Screen`], [`TreeItemId`], [`PickFilter`] and the
//! rest), the frame's inputs ([`UiFrameInputs`]), what a frame hands back
//! ([`UiFrameOutput`]) and the actions the UI asks for ([`UiCommand`]).
//!
//! The UI draws from these, and the host (`app_shell`) fills them and
//! applies the commands, seeing nothing of the UI but `UiLayer`. Nothing
//! here names the UI (`scripts/check-ui-boundary.mjs`).

mod commands;
mod inputs;
mod view;

pub use commands::{
    BodyEdit, ConfigEdit, EditCommand, FileCommand, PrintOn, SavedSectionAction, SettingsEdit,
    StartKind, UiCommand,
};
pub use inputs::{FileMarks, HoverCard, Physical, SelectionMass, UiFrameInputs};
pub use view::{
    BenchMenu, Example, HostCtxParams, InertiaReference, MarkArm, MarkEdit, MarkTool, MeasureBar,
    MenuLocal, NoteCardView, OpenTask, OtherPick, PickFilter, PropertyTab, RadialView, ServerBadge,
    TabInfo, TreeFeatureCommand, TreeItemId, UiHooks, ViewportMenu,
};

use core_document::{ToolBehavior, ToolDescriptor, WorkbenchId};
use render_wgpu::EguiSubmission;

/// Which top-level screen the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The launch page: new-document cards, recent files.
    Start,
    /// The modelling workspace.
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWorkbench(pub WorkbenchId);

/// The panels around the view as they were left: kept between runs.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PanelLayout {
    /// Each resizable panel's size across the window, by the panel's name.
    pub sizes: std::collections::BTreeMap<String, f32>,
    pub console_open: bool,
    pub assistant_open: bool,
    /// The property panel's page.
    pub property_page: PropertyTab,
    /// The tree's share of the left column.
    pub tree_share: Option<f32>,
}

#[derive(Debug, Clone, Default)]
pub struct ActiveTool {
    /// Set of active tool IDs. For Radio tools, only one per group is active.
    /// For Check tools, multiple can be active. For Action tools, this is cleared after handling.
    pub active_ids: std::collections::HashSet<String>,
}

impl ActiveTool {
    /// Activate `id` following `tool`'s behavior: Actions fire once, Checks
    /// toggle, Radios clear their group first.
    pub fn activate(&mut self, tools: &[ToolDescriptor], tool: &ToolDescriptor, id: &str) {
        let is_active = self.active_ids.contains(id);
        match tool.behavior {
            ToolBehavior::Action => {
                // Fire-and-forget: the host clears it after handling the input.
                self.active_ids.insert(id.to_owned());
            }
            ToolBehavior::Check => {
                if is_active {
                    self.active_ids.remove(id);
                } else {
                    self.active_ids.insert(id.to_owned());
                }
            }
            ToolBehavior::Radio => {
                if is_active {
                    self.active_ids.remove(id);
                } else {
                    match &tool.group {
                        Some(group) => self.active_ids.retain(|active_id| {
                            tools
                                .iter()
                                .find(|t| t.id == core_document::base_tool_id(active_id))
                                .map(|t| t.group.as_deref() != Some(group))
                                .unwrap_or(true)
                        }),
                        None => self.active_ids.clear(),
                    }
                    self.active_ids.insert(id.to_owned());
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Per-frame data (submission, viewport, tool/workbench state) plus the
/// list of actions the user triggered this frame.
pub struct UiFrameOutput {
    pub submission: EguiSubmission,
    /// egui's own repaint request: `Duration::ZERO` for "next frame please"
    /// (animations), a finite delay for timed repaints (caret blink), and
    /// effectively-infinite when content is static.
    pub repaint_delay: std::time::Duration,
    pub viewport: ViewportRect,
    /// The pane beside the viewport when the view is split.
    pub side_viewport: Option<ViewportRect>,
    pub active_tool: ActiveTool,
    pub active_workbench: ActiveWorkbench,
    pub commands: Vec<UiCommand>,
    /// A workbench task is open in the right panel after this frame.
    pub task_open: bool,
    /// The bench's floating card on screen this frame: its id, and
    /// whether its OK is held back.
    pub card: Option<(String, bool)>,
    /// The tree row under the pointer.
    pub tree_hovered: Option<TreeItemId>,
    /// The Print layout task is open after this frame.
    pub print_layout_open: bool,
    /// The marking tool in hand, while the Mark surface task is open.
    pub mark_arm: Option<crate::contract::MarkArm>,
}
