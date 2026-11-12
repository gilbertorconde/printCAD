//! The bench hooks: environment variables a scripted run sets so a
//! capture or a timing needs no clicks (the `printcad-bench-hooks` skill
//! lists them). They are read once, at start, into [`BenchHooks`];
//! [`BenchProgress`] keeps which have fired, and
//! [`PrintCadApp::drive_bench_hooks`] plays them each frame.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Instant;

use winit::event_loop::ActiveEventLoop;

use crate::PrintCadApp;
use crate::log_panel as app_log;
use crate::ui;

/// Frames after geometry arrives at which the bench click hook requests its
/// pick (the camera has settled by then), clicks, activates the bench tool
/// named by `PRINTCAD_BENCH_TOOL`, and reports what the rebuild made of it.
const BENCH_CLICK_PICK_FRAME: u32 = 180;
const BENCH_CLICK_FRAME: u32 = 191;
const BENCH_TOOL_FRAME: u32 = 192;
const BENCH_REPORT_FRAME: u32 = 300;
/// Idle frames `PRINTCAD_BENCH_EDIT` waits for before it opens its feature.
const BENCH_EDIT_IDLE_FRAMES: u32 = 30;

/// Every bench hook the run was started with, read from the environment
/// once.
#[derive(Debug, Default)]
pub(crate) struct BenchHooks {
    /// `PRINTCAD_OPEN_FILE`: files to import at start, `;`-separated.
    pub open_files: Vec<PathBuf>,
    /// `PRINTCAD_OPEN_SAME_TAB`: those files all land in one scene.
    pub open_same_tab: bool,
    /// `PRINTCAD_OPEN_DOC`: a document to open at start.
    pub open_doc: Option<PathBuf>,
    /// `PRINTCAD_BENCH_SKETCH`: the fixture scene to open (`pad`, or any
    /// other value for the plain sketch).
    pub sketch: Option<String>,
    /// `PRINTCAD_BENCH_ORBIT`: turn the camera every frame.
    pub orbit: bool,
    /// `PRINTCAD_BENCH_SPIN`: keep frames coming with no scene change.
    pub spin: bool,
    /// `PRINTCAD_EXIT_AFTER_MS`: quit through the real exit path then.
    pub exit_after_ms: Option<u64>,
    /// `PRINTCAD_BENCH_SELECT`: the body (index or name) or feature to
    /// select.
    pub select: Option<String>,
    /// `PRINTCAD_BENCH_ISOLATE`: isolate the first body whose name holds
    /// this.
    pub isolate: Option<String>,
    /// `PRINTCAD_BENCH_NOTE`: the note whose card opens.
    pub note: Option<u32>,
    /// `PRINTCAD_BENCH_EDIT`: the feature to open for editing.
    pub edit: Option<String>,
    /// `PRINTCAD_BENCH_CHAT`: a message for a chat with no agent.
    pub chat: Option<String>,
    /// `PRINTCAD_BENCH_POINTER`: where the pointer is held, as fractions
    /// of the viewport.
    pub pointer: Option<(f32, f32)>,
    /// `PRINTCAD_BENCH_TOOL`: the tool in hand, by its current id.
    pub tool: Option<String>,
    /// `PRINTCAD_BENCH_CLICK`: one selection click there, as fractions of
    /// the viewport (the middle when unreadable).
    pub click: Option<(f32, f32)>,
    /// `PRINTCAD_BENCH_PICTURE`: where the view's picture is written.
    pub picture: Option<PathBuf>,
    /// `PRINTCAD_BENCH_WINDOW`: where the window's picture is written.
    pub window: Option<PathBuf>,
    /// `PRINTCAD_BENCH_UI`: pointer steps played into the window.
    pub ui: Option<String>,
    /// `PRINTCAD_BENCH_WORKBENCH`: the bench to switch to.
    pub workbench: Option<String>,
    /// `PRINTCAD_BENCH_TASK`: the application task to open.
    pub task: Option<String>,
    /// `PRINTCAD_BENCH_REPAIR`: ask for every broken body's repair.
    pub repair: bool,
    /// `PRINTCAD_BENCH_CONVERT`: convert every mesh body (`refine` refines
    /// them too).
    pub convert: Option<String>,
    /// `PRINTCAD_BENCH_SECTION`: the clipping plane to turn on.
    pub section: Option<String>,
    /// `PRINTCAD_BENCH_PREFS`: the Preferences page (`<page>/<tab>`) to
    /// open.
    pub prefs: Option<String>,
}

/// The run's bench hooks, read on first use.
pub(crate) fn hooks() -> &'static BenchHooks {
    static HOOKS: OnceLock<BenchHooks> = OnceLock::new();
    HOOKS.get_or_init(|| BenchHooks::read(|name| std::env::var(name).ok()))
}

/// `<fx>,<fy>` as two numbers.
fn fractions(spec: &str) -> Option<(f32, f32)> {
    let (a, b) = spec.split_once(',')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

impl BenchHooks {
    /// The hooks `var` gives values for.
    fn read(var: impl Fn(&str) -> Option<String>) -> Self {
        let set = |name: &str| var(name).is_some();
        Self {
            open_files: var("PRINTCAD_OPEN_FILE")
                .map(|paths| {
                    paths
                        .split(';')
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default(),
            open_same_tab: set("PRINTCAD_OPEN_SAME_TAB"),
            open_doc: var("PRINTCAD_OPEN_DOC").map(PathBuf::from),
            sketch: var("PRINTCAD_BENCH_SKETCH"),
            orbit: set("PRINTCAD_BENCH_ORBIT"),
            spin: set("PRINTCAD_BENCH_SPIN"),
            exit_after_ms: var("PRINTCAD_EXIT_AFTER_MS").and_then(|v| v.parse().ok()),
            select: var("PRINTCAD_BENCH_SELECT"),
            isolate: var("PRINTCAD_BENCH_ISOLATE"),
            note: var("PRINTCAD_BENCH_NOTE").and_then(|n| n.parse().ok()),
            edit: var("PRINTCAD_BENCH_EDIT"),
            chat: var("PRINTCAD_BENCH_CHAT"),
            pointer: var("PRINTCAD_BENCH_POINTER").and_then(|p| fractions(&p)),
            tool: var("PRINTCAD_BENCH_TOOL")
                .map(|tool| core_document::renamed::command(&tool).into_owned()),
            click: var("PRINTCAD_BENCH_CLICK").map(|c| fractions(&c).unwrap_or((0.5, 0.5))),
            picture: var("PRINTCAD_BENCH_PICTURE").map(PathBuf::from),
            window: var("PRINTCAD_BENCH_WINDOW").map(PathBuf::from),
            ui: var("PRINTCAD_BENCH_UI"),
            workbench: var("PRINTCAD_BENCH_WORKBENCH"),
            task: var("PRINTCAD_BENCH_TASK"),
            repair: set("PRINTCAD_BENCH_REPAIR"),
            convert: var("PRINTCAD_BENCH_CONVERT"),
            section: var("PRINTCAD_BENCH_SECTION"),
            prefs: var("PRINTCAD_BENCH_PREFS"),
        }
    }

    /// Whether any hook is set: the run skips the start page and leaves
    /// the user's session as it was.
    pub(crate) fn any(&self) -> bool {
        let Self {
            open_files,
            open_same_tab,
            open_doc,
            sketch,
            orbit,
            spin,
            exit_after_ms,
            select,
            isolate,
            note,
            edit,
            chat,
            pointer,
            tool,
            click,
            picture,
            window,
            ui,
            workbench,
            task,
            repair,
            convert,
            section,
            prefs,
        } = self;
        !open_files.is_empty()
            || *open_same_tab
            || *orbit
            || *spin
            || *repair
            || exit_after_ms.is_some()
            || note.is_some()
            || pointer.is_some()
            || click.is_some()
            || [open_doc, picture, window].iter().any(|p| p.is_some())
            || [
                sketch, select, isolate, edit, chat, tool, ui, workbench, task, convert, section,
                prefs,
            ]
            .iter()
            .any(|v| v.is_some())
    }

    /// Whether the hooks keep frames coming on their own.
    pub(crate) fn animating(&self) -> bool {
        self.orbit || self.spin || self.exit_after_ms.is_some()
    }
}

/// How far the run's hooks have got.
pub(crate) struct BenchProgress {
    /// The start-up opens have run.
    pub open_fired: bool,
    pub select_fired: bool,
    pub isolate_fired: bool,
    /// The note's card is open.
    pub note_fired: bool,
    /// The feature is open for editing.
    pub edit_fired: bool,
    /// Frames the kernel has been idle with a document open.
    pub edit_idle: u32,
    /// Frames the picture has waited for the scene to settle; past its
    /// wait, the picture has been asked for.
    pub picture_frames: u32,
    pub task_fired: bool,
    pub workbench_fired: bool,
    pub repair_fired: bool,
    pub convert_fired: bool,
    pub refine_fired: bool,
    /// The clipping plane is set.
    pub section_fired: bool,
    /// The chat's message is sent.
    pub chat_fired: bool,
    /// Frames since the click hook saw geometry; drives its stages.
    pub click_frames: u32,
    /// Process start, for the exit hook.
    pub started: Instant,
}

impl Default for BenchProgress {
    fn default() -> Self {
        Self {
            open_fired: false,
            select_fired: false,
            isolate_fired: false,
            note_fired: false,
            edit_fired: false,
            edit_idle: 0,
            picture_frames: 0,
            task_fired: false,
            workbench_fired: false,
            repair_fired: false,
            convert_fired: false,
            refine_fired: false,
            section_fired: false,
            chat_fired: false,
            click_frames: 0,
            started: Instant::now(),
        }
    }
}

impl PrintCadApp {
    /// Play the bench hooks' part of this frame. False when the exit hook
    /// has ended the run.
    pub(crate) fn drive_bench_hooks(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let hooks = hooks();
        // Dev/bench hook: orbit continuously so frame measurements cover the
        // moving-camera case (the one the user feels).
        if hooks.orbit {
            self.session.view.camera.apply_rotate_delta(
                &crate::orientation_cube::RotateDelta {
                    axis: crate::orientation_cube::RotateAxis::ScreenY,
                    degrees: 0.5,
                },
                &self.user_settings.camera,
            );
        }

        // Dev/bench hook: quit through the real exit path after a delay, so
        // teardown can be timed on a loaded document without a dialog.
        if let Some(after_ms) = hooks.exit_after_ms
            && self.bench.started.elapsed().as_millis() as u64 >= after_ms
        {
            tracing::info!(target: "printcad.frame", "bench exit requested");
            self.wait_for_all_document_saves();
            self.keep_session_for_next_run();
            event_loop.exit();
            return false;
        }

        // Dev/bench hooks: import a STEP or open a document at startup
        // without a dialog.
        if !self.bench.open_fired {
            self.bench.open_fired = true;
            // Several files each import into a tab of their own, so a
            // capture can show the strip, or all into one scene.
            let detail = self.imports.last_detail.clone();
            for path in &hooks.open_files {
                if !hooks.open_same_tab {
                    self.ensure_fresh_tab();
                }
                self.import_step_at(path, detail.clone());
            }
            if let Some(path) = &hooks.open_doc {
                self.open_document_at(path.clone());
            }
            if hooks.sketch.is_some() {
                self.bench_open_sketch();
            }
            // Without a hook, the command line's files or the last run's.
            if crate::launch_screen() == crate::ui::Screen::Start {
                self.reopen_at_start();
            }
        }
        // Dev/bench hook: `PRINTCAD_BENCH_SELECT=<n or name>` selects the
        // n-th body, or the first whose name contains the text, once it has
        // geometry, so a capture can show the selection overlay without a
        // click.
        if !self.bench.select_fired
            && let Some(which) = &hooks.select
            && let Some(body) = match which.parse::<usize>() {
                Ok(n) => self.session.document.bodies().get(n),
                Err(_) => self
                    .session
                    .document
                    .bodies()
                    .iter()
                    .find(|b| b.name.contains(which.as_str())),
            }
            .map(|b| (b.id, b.name.clone()))
            && self.session.document.imported_geometry(body.0).is_some()
        {
            self.bench.select_fired = true;
            let row = match self.session.document.imported_object_for_body(body.0) {
                Some(node) => crate::ui::TreeItemId::ImportedObject(node),
                None => crate::ui::TreeItemId::Body(body.0),
            };
            self.apply_tree_selection(row);
            tracing::info!(target: "printcad.frame", "bench selected body {:?} `{}`", body.0, body.1);
        }
        // With no body of that name, the feature named exactly so, as a
        // click on its tree row would, once the document has bodies.
        let named_feature = |document: &core_document::Document, which: &str| {
            document
                .feature_tree()
                .all_nodes()
                .find(|(_, node)| node.name == which)
                .map(|(id, _)| *id)
        };
        if !self.bench.select_fired
            && let Some(which) = &hooks.select
            && !self.session.document.bodies().is_empty()
            && self.kernel_worker.in_flight() == 0
            && let Some(feature) = named_feature(&self.session.document, which)
        {
            self.bench.select_fired = true;
            self.apply_tree_selection(crate::ui::TreeItemId::Feature(feature));
            tracing::info!(target: "printcad.frame", "bench selected feature `{which}`");
        }
        // Dev/bench hook: `PRINTCAD_BENCH_ISOLATE=<name>` isolates the view
        // to the first body whose name contains the text, once it has
        // geometry and while nothing is isolated.
        if self.session.document.isolation().is_none()
            && let Some(which) = &hooks.isolate
            && let Some(body) = self
                .session
                .document
                .bodies()
                .iter()
                .find(|b| b.name.contains(which.as_str()))
                .map(|b| b.id)
            && self.session.document.imported_geometry(body).is_some()
            && !std::mem::replace(&mut self.bench.isolate_fired, true)
        {
            self.session.document.set_isolation(Some(vec![body]));
        }

        // Dev/bench hook: `PRINTCAD_BENCH_NOTE=<number>` arms the note tool
        // and opens the card on that note once its body has geometry, as a
        // click on it would, so a capture shows the card.
        if !self.bench.note_fired
            && let Some(number) = hooks.note
            && let Some((body, note)) = self.session.document.note_by_number(number)
            && self.session.document.imported_geometry(body).is_some()
        {
            let id = note.id;
            self.bench.note_fired = true;
            self.session.tools.note_tool = true;
            self.open_note_card(id);
        }

        // Dev/bench hook: `PRINTCAD_BENCH_EDIT=<name>` opens the first
        // feature of that name for editing, as a double click on its tree
        // row would (a sketch opens its editing session), once the kernel
        // has been idle for a while with the document open, so a capture
        // can show any document's sketch.
        let idle =
            self.kernel_worker.in_flight() == 0 && !self.session.document.bodies().is_empty();
        self.bench.edit_idle = if idle { self.bench.edit_idle + 1 } else { 0 };
        if !self.bench.edit_fired
            && let Some(name) = &hooks.edit
            && self.bench.edit_idle >= BENCH_EDIT_IDLE_FRAMES
        {
            self.bench.edit_fired = true;
            // The earliest in the history when several share the name.
            let found = self
                .session
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, node)| node.name == *name)
                .min_by_key(|(_, node)| node.seq)
                .map(|(id, _)| *id);
            match found {
                Some(id) => {
                    self.apply_tree_activation(crate::ui::TreeItemId::Feature(id));
                    tracing::info!(target: "printcad.frame", "bench editing `{name}`");
                }
                None => tracing::warn!(target: "printcad.frame", "bench edit: no feature `{name}`"),
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_CHAT=<message>` sends the message,
        // its `@` mentions taken from the list as a pick would take them,
        // in a chat with no agent behind it, once the kernel has been idle
        // a while (and `PRINTCAD_BENCH_SELECT` has selected), and the
        // assistant opens on it with `@` typed, so a capture shows chips and
        // the list.
        if !self.bench.chat_fired
            && self.bench.edit_idle >= BENCH_EDIT_IDLE_FRAMES
            && (self.bench.select_fired || hooks.select.is_none())
            && let Some(message) = &hooks.chat
        {
            self.bench.chat_fired = true;
            self.bench_chat(message.clone());
        }

        // Dev/bench hook: once `PRINTCAD_BENCH_EDIT` has opened its feature
        // (or with no such feature named) and the camera has settled,
        // `PRINTCAD_BENCH_POINTER=<fx>,<fy>` holds the pointer at that
        // fraction of the viewport, with the tool `PRINTCAD_BENCH_TOOL`
        // names in hand, so a capture shows a tool's cues, or the view's
        // hover, where the pointer is.
        if (self.bench.edit_fired || hooks.edit.is_none())
            && !self.session.view.camera.is_animating()
            && hooks.click.is_none()
            && let Some((fx, fy)) = hooks.pointer
        {
            let tool = hooks.tool.clone();
            if let Some(tool) = &tool {
                self.session.active_tool.active_ids.insert(tool.clone());
            }
            let vp = self.session.view.camera.viewport_info();
            let at = (fx * vp.2 as f32, fy * vp.3 as f32);
            self.input.cursor_in_viewport = Some(at);
            if !self.sketch_editing_active() {
                self.session.hover.feature = self.feature_under_cursor();
                self.session.hover.part = self.part_under_cursor();
            }
            let wb_id = self.active_workbench_id();
            self.call_workbench_input(
                &wb_id,
                &core_document::WorkbenchInputEvent::MouseMove { viewport_pos: at },
                tool.as_deref(),
            );
        }

        // Dev/bench hook: `PRINTCAD_BENCH_PICTURE=<path>` takes one picture
        // of the view once the scene has settled and writes it there.
        // `PRINTCAD_BENCH_WINDOW=<path>` does the same for the whole
        // window, the panels and menus with it.
        const PICTURE_SETTLE_FRAMES: u32 = 60;
        let scene = hooks.picture.is_some();
        let window = hooks.window.is_some();
        // Settling waits out a camera that is still turning to its view,
        // which a fast build would otherwise catch half way, and a scripted
        // click and its tool, an edit or a chat until they have had their
        // turn.
        // `PRINTCAD_BENCH_UI`'s steps play once the scene has settled, and
        // the picture waits for them.
        let settled = self.kernel_worker.in_flight() == 0
            && !self.session.view.camera.is_animating()
            && (self.bench.edit_fired || hooks.edit.is_none())
            && self.bench.edit_idle >= BENCH_EDIT_IDLE_FRAMES;
        let mut playing = false;
        let mut view_click = None;
        if let Some(gfx) = self.gfx.as_mut() {
            if settled {
                gfx.ui_layer.arm_bench_ui();
            }
            view_click = gfx.ui_layer.take_bench_view_click();
            playing = gfx.ui_layer.bench_ui_pending();
        }
        if let Some(at) = view_click {
            self.bench_view_click(at);
        }
        if playing {
            self.redraw_needed = true;
        }
        let clicking = playing
            || hooks.click.is_some() && self.bench.click_frames <= BENCH_REPORT_FRAME
            || hooks.edit.is_some() && !self.bench.edit_fired
            || hooks.chat.is_some() && !self.bench.chat_fired;
        if (scene || window)
            && self.bench.picture_frames <= PICTURE_SETTLE_FRAMES
            && self.kernel_worker.in_flight() == 0
            && !self.session.view.camera.is_animating()
            && !clicking
        {
            self.bench.picture_frames += 1;
            self.redraw_needed = true;
            if self.bench.picture_frames > PICTURE_SETTLE_FRAMES
                && let Some(gfx) = self.gfx.as_mut()
            {
                if scene {
                    gfx.renderer
                        .request_capture(render_wgpu::CaptureKind::Scene);
                }
                if window {
                    gfx.renderer
                        .request_capture(render_wgpu::CaptureKind::Window);
                }
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_WORKBENCH=<bench id>` switches to
        // that bench once the document has a body, as the switcher would,
        // so a capture shows its toolbar and `PRINTCAD_BENCH_TOOL` reaches
        // its tools.
        if !self.bench.workbench_fired
            && let Some(id) = &hooks.workbench
            && !self.session.document.bodies().is_empty()
        {
            self.bench.workbench_fired = true;
            let target = crate::WorkbenchId::from(id.as_str());
            if self.registry.descriptor(&target).is_some() {
                self.switch_workbench_for_flow(target);
            } else {
                tracing::warn!(target: "printcad.frame", "no bench `{id}` to switch to");
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_TASK=appearance|placement|history|
        // texture|print_layout|array|note|mark` opens that task of the
        // application's on the first body (its first feature, for history;
        // its first note, or a new one; the marking of a scan) once it has
        // geometry, as its menu entry would, so a capture shows the task
        // panel.
        if !self.bench.task_fired
            && let Some(which) = &hooks.task
            && let Some(body) = self.session.document.bodies().first().map(|b| b.id)
            && self.session.document.imported_geometry(body).is_some()
            && let Some(gfx) = self.gfx.as_mut()
        {
            self.bench.task_fired = true;
            let first = self
                .session
                .document
                .feature_tree()
                .all_nodes()
                .filter(|(_, n)| n.body == Some(body))
                .min_by_key(|(id, n)| (n.seq, **id))
                .map(|(id, _)| *id);
            let open = match which.as_str() {
                "placement" => Some(ui::OpenTask::Placement(body)),
                "history" => first.map(ui::OpenTask::History),
                "texture" => Some(ui::OpenTask::Texture(body, None)),
                "print_layout" => Some(ui::OpenTask::PrintLayout),
                "array" => Some(ui::OpenTask::Array(body)),
                "note" => Some(match self.session.document.body_notes(body).first() {
                    Some(note) => ui::OpenTask::EditNote(note.id),
                    None => ui::OpenTask::AddNote(body, None),
                }),
                "mark" => Some(ui::OpenTask::Mark(body)),
                _ => Some(ui::OpenTask::Appearance(body, None)),
            };
            if let Some(open) = open {
                gfx.ui_layer
                    .open_task(&self.session.document, open, self.session.tab);
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_REPAIR=1` asks for the repair of
        // every body the checker calls broken, once, as the tree's menu
        // would, so a run shows the repair land.
        if !self.bench.repair_fired && hooks.repair {
            let broken: Vec<_> = self
                .session
                .document
                .imported_geometries()
                .filter(|(_, g)| g.health.as_ref().is_some_and(|h| h.is_broken()))
                .map(|(body, _)| *body)
                .collect();
            if !broken.is_empty() {
                self.bench.repair_fired = true;
                self.apply_ui_commands(vec![ui::UiCommand::RepairShapes(broken)], event_loop);
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_CONVERT=1` asks for every mesh
        // body to become a solid, once, as the tree's menu would; `=refine`
        // then asks for every converted solid's refine once it lands.
        if !self.bench.convert_fired && hooks.convert.is_some() {
            let meshes: Vec<_> = self
                .session
                .document
                .bodies()
                .iter()
                .map(|b| b.id)
                .filter(|b| self.session.document.is_mesh_body(*b))
                .collect();
            if !meshes.is_empty() {
                self.bench.convert_fired = true;
                self.apply_ui_commands(vec![ui::UiCommand::ConvertToSolid(meshes)], event_loop);
            }
        }

        if !self.bench.refine_fired && hooks.convert.as_deref() == Some("refine") {
            let faceted: Vec<_> = self
                .session
                .document
                .bodies()
                .iter()
                .map(|b| b.id)
                .filter(|b| self.session.document.can_refine(*b))
                .collect();
            if !faceted.is_empty() {
                self.bench.refine_fired = true;
                self.apply_ui_commands(vec![ui::UiCommand::RefineShapes(faceted)], event_loop);
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_CLICK=<fx>,<fy>` makes one selection
        // click at that fraction of the viewport once the first body has
        // geometry, and logs what the click saw and what it selected. The
        // camera snaps to a corner view first and settles before the pick is
        // requested, then the readback gets a few frames to land. With
        // `PRINTCAD_BENCH_TOOL=<tool id>` the tool then runs on that
        // selection, as a toolbar click would, and every feature's error is
        // logged once the rebuild has had its turn.
        if let Some((fx, fy)) = hooks.click
            && self.bench.click_frames <= BENCH_REPORT_FRAME
            && self
                .session
                .document
                .bodies()
                .first()
                .is_some_and(|b| self.session.document.imported_geometry(b.id).is_some())
        {
            self.bench.click_frames += 1;
            self.redraw_needed = true;
            let vp = self.session.view.camera.viewport_info();
            let cx = fx * vp.2 as f32;
            let cy = fy * vp.3 as f32;
            match self.bench.click_frames {
                1 => self.session.view.camera.snap_to_view(
                    crate::orientation_cube::CameraSnapView::FrontTopRight,
                    &self.user_settings.camera,
                ),
                BENCH_CLICK_PICK_FRAME..BENCH_CLICK_FRAME => {
                    self.input.cursor_in_viewport = Some((cx, cy));
                    if let Some(gfx) = self.gfx.as_mut() {
                        gfx.renderer
                            .request_pick((vp.0 + cx) as u32, (vp.1 + cy) as u32);
                    }
                }
                BENCH_CLICK_FRAME => {
                    tracing::info!(
                        target: "printcad.frame",
                        "bench click at ({cx}, {cy}) of {:?}: body {:?} at {:?}, triangle {:?}, edge {:?}, face {:?}",
                        (vp.2, vp.3),
                        self.session.hover.body,
                        self.session.hover.world_pos,
                        self.session.hover.triangle,
                        self.session.hover.edge,
                        self.session.hover.face.as_ref().map(|f| f.face),
                    );
                    self.toggle_body_under_cursor_selection();
                    tracing::info!(
                        target: "printcad.frame",
                        "bench click selected: body {:?}, face {:?}, face highlight {}, edges {}",
                        self.session.selection.body,
                        self.session.selection.last_face_hit,
                        self.session.selection.face_highlight.is_some(),
                        self.session.selection.edges.len(),
                    );
                }
                BENCH_TOOL_FRAME => {
                    if let Some(tool) = hooks.tool.clone() {
                        tracing::info!(target: "printcad.frame", "bench tool {tool}");
                        self.session.active_tool.active_ids.insert(tool);
                    }
                }
                BENCH_REPORT_FRAME => {
                    for (id, node) in self.session.document.feature_tree().all_nodes() {
                        tracing::info!(
                            target: "printcad.frame",
                            "bench feature {:?} `{}` dirty {} error {:?}",
                            id,
                            node.name,
                            node.dirty,
                            node.error,
                        );
                    }
                }
                _ => {}
            }
        }

        // Dev/bench hook: `PRINTCAD_BENCH_SECTION=<aim>[@<offset>][!][^<way>][~<body>]` turns
        // the clipping plane on once the first body has geometry (after
        // `PRINTCAD_BENCH_CLICK`'s click, when there is one), aimed as the
        // clip pill would: `x`, `y`, `z`, `view`, `face` (the selected flat
        // face, else waiting for a click), `points:x,y,z;x,y,z;x,y,z` or
        // `saved:<name>` (a section the document keeps),
        // moved `offset` mm along its normal, `!` keeping the other side,
        // `^0` or `^1` adding a wedge facing that way, the body of that
        // name left whole.
        if !self.bench.section_fired
            && let Some(spec) = &hooks.section
            && self
                .session
                .document
                .bodies()
                .first()
                .is_some_and(|b| self.session.document.imported_geometry(b.id).is_some())
            && (hooks.click.is_none() || self.bench.click_frames > BENCH_CLICK_FRAME)
        {
            self.bench.section_fired = true;
            self.bench_section(spec);
        }

        true
    }

    /// A left click in the viewport at fraction `at` of it, as the
    /// viewport's own input hands it to the active bench with the tool in
    /// hand.
    fn bench_view_click(&mut self, at: egui::Pos2) {
        let vp = self.session.view.camera.viewport_info();
        let viewport_pos = (at.x * vp.2 as f32, at.y * vp.3 as f32);
        self.input.cursor_in_viewport = Some(viewport_pos);
        let tool = self.session.active_tool.active_ids.iter().next().cloned();
        let wb_id = self.active_workbench_id();
        let button = core_document::MouseButton::Left;
        for event in [
            core_document::WorkbenchInputEvent::MouseMove { viewport_pos },
            core_document::WorkbenchInputEvent::MousePress {
                button,
                viewport_pos,
            },
            core_document::WorkbenchInputEvent::MouseRelease {
                button,
                viewport_pos,
            },
        ] {
            self.call_workbench_input(&wb_id, &event, tool.as_deref());
        }
        self.redraw_needed = true;
    }

    /// A body with a small constrained sketch, opened for editing, so the
    /// sketcher can be exercised without clicking; `pad` pads it and opens
    /// the pad for editing, `pocket` pockets the pad's top too and opens
    /// the pocket.
    fn bench_open_sketch(&mut self) {
        self.create_new_body();
        let scene = bench_fixtures::Scene::named(
            crate::app::bench_hooks::hooks()
                .sketch
                .as_deref()
                .unwrap_or_default(),
        );
        match bench_fixtures::open_sketch_scene(
            &mut self.session.document,
            self.session.active_body_id,
            scene,
        ) {
            Ok(handles) => {
                self.apply_tree_activation(crate::ui::TreeItemId::Feature(handles.feature));
            }
            Err(err) => app_log::error(format!("bench scene: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hooks_read_their_values_and_any_one_skips_the_start_page() {
        let read = |set: &[(&str, &str)]| {
            let set: Vec<(String, String)> = set
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            BenchHooks::read(|name| set.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()))
        };
        assert!(!read(&[]).any());
        assert!(read(&[("PRINTCAD_BENCH_PREFS", "Display")]).any());
        let hooks = read(&[
            ("PRINTCAD_OPEN_FILE", "a.step;;b.stl"),
            ("PRINTCAD_BENCH_CLICK", "nowhere"),
            ("PRINTCAD_BENCH_POINTER", "0.25,0.75"),
            ("PRINTCAD_BENCH_TOOL", "part.pad"),
            ("PRINTCAD_EXIT_AFTER_MS", "2000"),
        ]);
        assert_eq!(
            hooks.open_files,
            [PathBuf::from("a.step"), PathBuf::from("b.stl")]
        );
        assert_eq!(hooks.click, Some((0.5, 0.5)), "unreadable is the middle");
        assert_eq!(hooks.pointer, Some((0.25, 0.75)));
        assert_eq!(
            hooks.tool.as_deref(),
            Some("design.pad"),
            "old ids read as new"
        );
        assert!(hooks.animating());
    }
}
