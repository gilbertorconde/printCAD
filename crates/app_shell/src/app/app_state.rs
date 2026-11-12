//! The app's own state outside its tabs, in groups a module can borrow on
//! their own while the rest of the app is borrowed elsewhere.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use uuid::Uuid;

/// What the 1 s frame log and the status bar's FPS read.
pub(crate) struct FrameStats {
    pub last_frame_time: Option<Instant>,
    pub current_fps: f32,
    pub fps_accum_time: f32,
    pub fps_frame_count: u32,
    /// Rolling per-phase frame cost, emitted once a second alongside the FPS
    /// counter (target `printcad.frame`): (ui ms, render ms, frames).
    pub frame_phase_accum: (f32, f32, u32),
    /// Scene redraws in the current 1 s window and the last completed one.
    /// The scene is cached between changes, so this diverges from fps: it
    /// is what "the parts' fps" actually is.
    pub scene_redraw_accum: u32,
    pub scene_redraws_per_s: u32,
    /// How often the status-bar text changed this second, and what it last
    /// read. A readable status changes a few times a second at most; a slot
    /// being overwritten by twenty threads changes every frame.
    pub status_changes_accum: u32,
    pub last_status_text: Option<String>,
    /// Why the last frame scheduled another: (input, work, animating,
    /// egui-zero-delay). Surfaced in the 1 s frame log while diagnosing
    /// wake-loop bugs.
    pub last_wake_reason: (bool, bool, bool, bool),
    /// The render loop decided to sleep and painted one closing frame whose
    /// FPS reads "idle": a frozen number would look like a measurement.
    pub fps_display_idle: bool,
    /// Exponentially smoothed frame time (seconds). Updated every rendered
    /// frame, so the FPS display is live from the first measured interval
    /// after a wake, with no batching delay. `None` right after a sleep; the
    /// next frame's dt seeds it with a real measurement.
    pub smoothed_frame_s: Option<f32>,
}

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            last_frame_time: None,
            current_fps: 0.0,
            fps_accum_time: 0.0,
            fps_frame_count: 0,
            frame_phase_accum: (0.0, 0.0, 0),
            scene_redraw_accum: 0,
            scene_redraws_per_s: 0,
            status_changes_accum: 0,
            last_status_text: None,
            last_wake_reason: (false, false, false, false),
            fps_display_idle: false,
            smoothed_frame_s: None,
        }
    }
}

/// A 1 s window of frames, averaged for the frame log.
pub(crate) struct FrameSecond {
    pub ui_ms: f32,
    pub render_ms: f32,
    pub scene_redraws: u32,
    pub status_changes: u32,
}

impl FrameStats {
    /// Count a frame rendered at `now`: the time since the last rendered
    /// frame, and the second's averages when a 1 s window with frames in
    /// it closes.
    pub fn frame_at(&mut self, now: Instant) -> (f32, Option<FrameSecond>) {
        let mut second = None;
        let dt_secs = if let Some(last) = self.last_frame_time {
            let dt = (now - last).as_secs_f32();
            // Waking from an on-demand sleep: the gap is idle time, not a
            // slow frame. Drop the seed so the next dt restarts the average;
            // the displayed number stays live per frame below.
            if dt > 0.5 {
                self.fps_accum_time = 0.0;
                self.fps_frame_count = 0;
                self.smoothed_frame_s = None;
            } else if dt > 0.0 {
                // The display updates every frame from a smoothed frame time:
                // a real number one frame after waking, without the jitter
                // of raw per-frame values. The 1 s accumulator below only
                // paces the log line.
                let sft = match self.smoothed_frame_s {
                    None => dt,
                    Some(prev) => prev * 0.85 + dt * 0.15,
                };
                self.smoothed_frame_s = Some(sft);
                self.current_fps = 1.0 / sft.max(1e-6);

                self.fps_accum_time += dt;
                self.fps_frame_count += 1;
                if self.fps_accum_time >= 1.0 {
                    self.fps_accum_time = 0.0;
                    self.fps_frame_count = 0;
                    let (ui_ms, render_ms, frames) = self.frame_phase_accum;
                    if frames > 0 {
                        second = Some(FrameSecond {
                            ui_ms: ui_ms / frames as f32,
                            render_ms: render_ms / frames as f32,
                            scene_redraws: self.scene_redraw_accum,
                            status_changes: self.status_changes_accum,
                        });
                    }
                    self.frame_phase_accum = (0.0, 0.0, 0);
                    self.scene_redraws_per_s = self.scene_redraw_accum;
                    self.scene_redraw_accum = 0;
                    self.status_changes_accum = 0;
                }
            }
            dt
        } else {
            0.016 // ~60fps default for first frame
        };
        self.last_frame_time = Some(now);
        (dt_secs, second)
    }

    /// Count a change of the status bar's text.
    pub fn note_status(&mut self, status: &Option<String>) {
        if *status != self.last_status_text {
            self.status_changes_accum += 1;
            self.last_status_text = status.clone();
        }
    }
}

/// The graphics device the renderer runs on, as Preferences shows it.
#[derive(Default)]
pub(crate) struct GpuInfo {
    pub name: Option<String>,
    pub api: Option<&'static str>,
    pub available: Vec<String>,
}

/// Where the pointer is and what the keyboard and buttons hold.
pub(crate) struct InputState {
    pub cursor_in_viewport: Option<(f32, f32)>,
    /// The cursor over the split view's side pane, in that pane's pixels;
    /// held while a gesture begun there lasts.
    pub cursor_in_side: Option<(f32, f32)>,
    /// A press on the side pane holds the pointer there until released.
    pub side_gesture: bool,
    /// When user input last arrived. Frames keep coming for a short tail
    /// after input so egui reactions and pick readbacks land, then the loop
    /// sleeps until the next event (render on demand).
    pub last_input_time: Option<Instant>,
    /// Pressed-mouse-button count; nonzero suppresses undo boundaries.
    pub mouse_buttons_down: u32,
    /// Which of the left, right and middle buttons are down, whoever took
    /// their presses: the camera lets go of a drag whose button is not.
    pub held_buttons: [bool; 3],
    /// Latest keyboard modifiers from `WindowEvent::ModifiersChanged`.
    pub modifiers: winit::keyboard::ModifiersState,
    /// The paste key went down since the last frame.
    pub paste_pressed: bool,
    /// The window has the keyboard: chats notify only while it has not.
    pub window_focused: bool,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            cursor_in_viewport: None,
            cursor_in_side: None,
            side_gesture: false,
            last_input_time: None,
            mouse_buttons_down: 0,
            held_buttons: [false; 3],
            modifiers: winit::keyboard::ModifiersState::default(),
            paste_pressed: false,
            window_focused: true,
        }
    }
}

/// Imports in flight, by the path the kernel worker echoes back.
pub(crate) struct Imports {
    /// Which tab asked for each STEP import in flight.
    pub owner: HashMap<PathBuf, Uuid>,
    /// Imports asked for as scans: their mesh bodies are marked so once they land.
    pub scans: HashSet<PathBuf>,
    /// The units in-flight mesh imports were read in, when the file does not say.
    pub units: HashMap<PathBuf, kernel_api::LengthUnit>,
    /// Reuse the last confirmed import options when opening the dialog again.
    pub last_detail: kernel_api::TessellationSettings,
}

impl Imports {
    pub fn new(last_detail: kernel_api::TessellationSettings) -> Self {
        Self {
            owner: HashMap::new(),
            scans: HashSet::new(),
            units: HashMap::new(),
            last_detail,
        }
    }
}

/// File dialogs, exports and the files the app remembers.
pub(crate) struct FileWork {
    /// Pending file dialog result from its background thread.
    pub dialog_rx: Option<std::sync::mpsc::Receiver<crate::app::doc_io::FileDialogResult>>,
    /// An export being written on its own thread.
    pub export_rx: Option<std::sync::mpsc::Receiver<crate::app::export::ExportOutcome>>,
    /// The export options last confirmed, which the dialog opens on.
    pub last_export: crate::app::export::ExportDraft,
    /// Recently opened documents and the last dialog directory.
    pub recent: settings::recent::RecentStore,
    /// The files of the tabs closed, the latest last: what Reopen closed
    /// tab opens again.
    pub closed_files: Vec<PathBuf>,
    /// A picture of the view that arrived from the renderer, to be saved.
    pub picture: Option<render_wgpu::CapturedImage>,
}

impl FileWork {
    pub fn new(recent: settings::recent::RecentStore) -> Self {
        Self {
            dialog_rx: None,
            export_rx: None,
            last_export: Default::default(),
            recent,
            closed_files: Vec::new(),
            picture: None,
        }
    }
}

/// Stable renderer ids for the overlay submissions the host draws, so the
/// renderer's per-body cache works for overlays too.
pub(crate) struct OverlayIds {
    /// The face-highlight overlay slot.
    pub face_highlight: Uuid,
    /// The hovered face's overlay.
    pub face_hover: Uuid,
    /// The overlay slot of the body a tree row under the pointer stands for.
    pub tree_hover: Uuid,
    /// The whole-body selection overlay.
    pub body_highlight: Uuid,
    /// The hovered-edge and selected-edges outlines.
    pub edge_hover: Uuid,
    pub edge_select: Uuid,
    /// The print bed's lines.
    pub print_bed: Uuid,
    /// Index-stable ids for workbench overlay meshes (slot i -> pool[i]).
    pub pool: Vec<Uuid>,
}

impl Default for OverlayIds {
    fn default() -> Self {
        Self {
            face_highlight: Uuid::new_v4(),
            face_hover: Uuid::new_v4(),
            tree_hover: Uuid::new_v4(),
            body_highlight: Uuid::new_v4(),
            edge_hover: Uuid::new_v4(),
            edge_select: Uuid::new_v4(),
            print_bed: Uuid::new_v4(),
            pool: Vec::new(),
        }
    }
}

impl OverlayIds {
    /// The id of workbench overlay slot `index`, the pool grown to it.
    pub fn slot(&mut self, index: usize) -> Uuid {
        while self.pool.len() <= index {
            self.pool.push(Uuid::new_v4());
        }
        self.pool[index]
    }
}

/// Meshes the host makes for the scene beside the bodies', kept by what
/// they were made from.
pub(crate) struct SceneMeshes {
    /// The print bed's line mesh, keyed by the settings it was built from.
    pub print_bed: Option<(u64, Arc<kernel_api::TriMesh>)>,
    /// The regions passive features shade, meshed by the kernel, by
    /// feature and region with the revision they were meshed at.
    pub regions: HashMap<(core_document::FeatureId, usize), (u64, Arc<kernel_api::TriMesh>)>,
    pub origin_planes: crate::app::scene_guides::OriginPlanes,
}

impl Default for SceneMeshes {
    fn default() -> Self {
        Self {
            print_bed: None,
            regions: HashMap::new(),
            origin_planes: crate::app::scene_guides::OriginPlanes::new(),
        }
    }
}

/// Questions on screen that something waits on.
#[derive(Default)]
pub(crate) struct Questions {
    /// The unsaved-edits question, and what waits for it.
    pub unsaved: Option<crate::app::unsaved::Asking>,
    /// A save over a file from another version, waiting for the user.
    pub save_over: Option<crate::app::file_safety::SaveOver>,
    /// Tabs whose edits the user answered for while quitting.
    pub quit_answered: HashSet<Uuid>,
}

/// What the last run left and what this one keeps for the next.
pub(crate) struct KeptRun {
    /// What the last run left: its open documents, window and panels.
    pub last_session: crate::app::session_store::KeptSession,
    /// The session as this run keeps it while it goes on.
    pub keeping: crate::app::session_store::SessionKeeping,
    /// Files named on the command line, opened at start in place of the
    /// last session's.
    pub launch_files: Vec<PathBuf>,
}

impl KeptRun {
    pub fn new(last_session: crate::app::session_store::KeptSession) -> Self {
        Self {
            last_session,
            keeping: Default::default(),
            launch_files: Vec::new(),
        }
    }
}

/// The script thread, the runs it has and the scripts it can run.
pub(crate) struct Scripting {
    /// The thread the console and scripts run on; its engine keeps the
    /// console's globals between lines.
    pub thread: scripting::ScriptThread,
    /// Runs submitted and not finished, oldest (the running one) first.
    pub runs: std::collections::VecDeque<crate::app::scripts::ScriptRun>,
    /// The running script's `doc.rebuild`, waiting on the kernel.
    pub rebuild: Option<crate::app::scripts::RebuildWait>,
    /// A recording of what is done through the UI, while one is on.
    pub recording: Option<scripting::Recorder>,
    /// Script files picked in a dialog, to run once it answers.
    pub to_run: Vec<PathBuf>,
    /// The scripts folder's scripts, and when it was last read.
    pub library: Vec<crate::script_library::ScriptEntry>,
    pub library_read: Option<Instant>,
    /// A script printed or failed: the console opens to show it.
    pub console_attention: bool,
}

impl Scripting {
    /// Read the scripts folder again, every couple of seconds while frames
    /// run, so a script saved in an editor shows up without a restart.
    pub fn refresh_library(&mut self) {
        let due = self
            .library_read
            .is_none_or(|at| at.elapsed() > std::time::Duration::from_secs(2));
        if !due {
            return;
        }
        self.library_read = Some(Instant::now());
        self.library = settings::scripts_dir()
            .map(|dir| crate::script_library::scan(&dir))
            .unwrap_or_default();
    }

    /// The scripts folder changed: read it again on the next frame.
    pub fn library_changed(&mut self) {
        self.library_read = None;
    }

    pub fn new(thread: scripting::ScriptThread) -> Self {
        Self {
            thread,
            runs: Default::default(),
            rebuild: None,
            recording: None,
            to_run: Vec::new(),
            library: Vec::new(),
            library_read: None,
            console_attention: false,
        }
    }
}

/// Autosave's copies of edited tabs and what a crash left.
pub(crate) struct Autosave {
    /// When the documents were last autosaved.
    pub at: Instant,
    /// Each tab's document as last autosaved, by its mutation seq.
    pub saved: HashMap<Uuid, u64>,
    /// Autosave writes on their threads, joined before an exit.
    pub jobs: Vec<crate::platform::Job>,
    /// Copies a crash left, offered back on the start page.
    pub recoverable: Vec<crate::app::recovery::Recoverable>,
}

impl Autosave {
    pub fn new(recoverable: Vec<crate::app::recovery::Recoverable>) -> Self {
        Self {
            at: Instant::now(),
            saved: HashMap::new(),
            jobs: Vec::new(),
            recoverable,
        }
    }
}

/// The MCP server, the chats with agents and what they wait on.
pub(crate) struct Agents {
    /// The MCP server agents reach the document through.
    pub mcp: Option<crate::app::mcp::McpServer>,
    /// Changes agents asked for, waiting for the user's OK.
    pub approvals: Vec<crate::app::mcp::Approval>,
    /// `screen` calls waiting for the next picture of the view.
    pub screen_waiting: Vec<std::sync::mpsc::Sender<agents::mcp::ToolAnswer>>,
    /// An agent needs the user: the assistant panel opens.
    pub assistant_attention: bool,
    /// The chats with agents, in the order they were opened.
    pub chats: Vec<crate::app::chats::Chat>,
    /// How many chats this session has opened, for the next one's name.
    pub chats_made: usize,
    /// The closed chats the assistant's history lists, read again whenever
    /// one closes, reopens or is forgotten, and when the list opens.
    pub history: Vec<crate::app::chats::PastChat>,
    /// Chats closed on documents not saved, by tab: kept for the run, and
    /// moved into the store when the document is first saved.
    pub closed_unsaved: Vec<(Uuid, crate::app::chat_store::SavedChat)>,
    /// Wakes the loop from another thread.
    pub waker: Arc<dyn Fn() + Send + Sync>,
}

impl Agents {
    pub fn new(waker: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            mcp: None,
            approvals: Vec::new(),
            screen_waiting: Vec::new(),
            assistant_attention: false,
            chats: Vec::new(),
            chats_made: 0,
            history: Vec::new(),
            closed_unsaved: Vec::new(),
            waker,
        }
    }
}

/// Workbench packages and the looks the app makes on the network for
/// them and for itself.
#[derive(Default)]
pub(crate) struct Packages {
    /// The workbench packages found at start, and those installed, updated
    /// or removed while the app runs.
    pub installed: Vec<workbenches::PackageStatus>,
    /// Package installs, update checks and updates running on threads.
    pub work: crate::app::packages::PackageWork,
    /// The workbench stores' indexes, and which are being fetched.
    pub store: crate::app::packages::StoreView,
    /// What the last look for a newer printCAD found.
    pub release_check: crate::app::updates::ReleaseCheck,
}
