//! A package driven without a window, the same way every run: pointer
//! steps in viewport pixels under a fixed camera, keys, tools, the
//! floating card's fields, OK and Cancel, tab switches, focus loss and
//! unload, each with what the app does around it. The undo history
//! follows the app's rules: a step closes whenever no button is held and
//! no card is open, the step before a card closes as it opens, and the
//! card's OK closes one step while Cancel takes back what was written.
//! See `docs/PLUGINS.md` › Testing a package.

use bench_api::{Capabilities, FloatingCard, Frame, PanelEvent};
use core_document::history::OpJournal;
use core_document::runtime::{KeyCode, MouseButton};
use core_document::{
    CommandArgs, CommandResult, Document, Workbench, WorkbenchInputEvent, WorkbenchRuntimeContext,
};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::{Package, WasmWorkbench};

/// The view every step is seen through: an 800 × 600 viewport looking
/// straight down from 100 mm above the origin, orthographic at 10 pixels
/// a millimetre. The view's middle, (400, 300), is over the origin; x
/// grows to the right and y up the screen.
const CAMERA: ([f32; 3], [f32; 3]) = ([0.0, 0.0, 100.0], [0.0; 3]);
const VIEWPORT: (u32, u32, u32, u32) = (0, 0, 800, 600);
const VIEW_PROJ: [[f32; 4]; 4] = [
    [1.0 / 40.0, 0.0, 0.0, 0.0],
    [0.0, -1.0 / 30.0, 0.0, 0.0],
    [0.0, 0.0, -1.0 / 200.0, 0.0],
    [0.0, 0.0, 0.5, 1.0],
];

/// The viewport pixel over the point (`x`, `y`) of the ground plane.
pub fn pixel(x: f32, y: f32) -> (f32, f32) {
    (400.0 + 10.0 * x, 300.0 - 10.0 * y)
}

/// One tab: its document, undo history and what the package kept of it
/// while off screen.
struct Tab {
    document: Document,
    journal: OpJournal,
    kept: Option<Box<dyn std::any::Any + Send>>,
    card: Option<String>,
}

impl Tab {
    fn new(name: &str) -> Self {
        Self {
            document: Document::new(name),
            journal: OpJournal::new(100),
            kept: None,
            card: None,
        }
    }
}

/// A loaded package with tabs of its own; tab 0 is on screen at first.
pub struct Harness {
    bench: Option<WasmWorkbench>,
    tabs: Vec<Option<Tab>>,
    on_screen: usize,
    tool: Option<String>,
    held: Vec<MouseButton>,
    logs: Vec<String>,
}

impl Harness {
    /// The package in `dir` (a folder holding `bench.toml` and
    /// `bench.wasm`), loaded with `granted` and active in one empty tab.
    pub fn open(dir: &Path, granted: &Capabilities) -> Result<Self, String> {
        Self::load(&Package::read(dir)?, granted)
    }

    pub fn load(package: &Package, granted: &Capabilities) -> Result<Self, String> {
        let mut harness = Self {
            bench: Some(crate::load(package, granted)?),
            tabs: vec![Some(Tab::new("tab 0"))],
            on_screen: 0,
            tool: None,
            held: Vec::new(),
            logs: Vec::new(),
        };
        harness.with(|bench, ctx| bench.on_activate(ctx));
        Ok(harness)
    }

    /// Load `package` again in place of the one running, as an update
    /// does: the old one is turned off and every tab's kept state dropped.
    pub fn reload(&mut self, package: &Package, granted: &Capabilities) -> Result<(), String> {
        self.unload();
        self.bench = Some(crate::load(package, granted)?);
        self.with(|bench, ctx| bench.on_activate(ctx));
        Ok(())
    }

    /// Turn the package off: deactivated on screen, what tabs kept of it
    /// dropped, an open card gone without a close.
    pub fn unload(&mut self) {
        self.with(|bench, ctx| bench.on_deactivate(ctx));
        self.bench = None;
        for tab in self.tabs.iter_mut().flatten() {
            tab.kept = None;
            tab.card = None;
        }
        self.held.clear();
    }

    fn tab(&mut self) -> &mut Tab {
        self.tabs[self.on_screen]
            .as_mut()
            .expect("the tab on screen is open")
    }

    pub fn document(&self) -> &Document {
        &self.tabs[self.on_screen].as_ref().expect("open").document
    }

    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.tab().document
    }

    /// `f` with the bench and a context for the tab on screen; nothing
    /// once unloaded. What the package logged is kept for [`logs`].
    ///
    /// [`logs`]: Self::logs
    pub fn with<R>(
        &mut self,
        f: impl FnOnce(&mut WasmWorkbench, &mut WorkbenchRuntimeContext) -> R,
    ) -> Option<R> {
        let bench = self.bench.as_mut()?;
        let tab = self.tabs[self.on_screen].as_mut()?;
        let mut ctx = WorkbenchRuntimeContext::new(&mut tab.document, CAMERA.0, CAMERA.1, VIEWPORT);
        ctx.view_proj = Some(VIEW_PROJ);
        let out = f(bench, &mut ctx);
        self.logs
            .extend(ctx.drain_logs().into_iter().map(|l| l.message));
        Some(out)
    }

    /// What the package logged since last asked.
    pub fn logs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.logs)
    }

    /// The package's frame for the tab on screen.
    pub fn frame(&mut self) -> Frame {
        self.with(|bench, ctx| bench.frame(ctx)).unwrap_or_default()
    }

    /// The card on screen.
    pub fn card(&mut self) -> Option<FloatingCard> {
        self.with(|bench, ctx| bench.floating_card(ctx)).flatten()
    }

    /// What the app does after each step: a card that opens closes the
    /// step before it; with no card and no button held, a step closes.
    fn settle(&mut self) {
        let shown = self.card().map(|c| c.id);
        let idle = self.held.is_empty();
        let tab = self.tab();
        let opens = shown.is_some() && shown != tab.card;
        tab.card = shown;
        if opens || (tab.card.is_none() && idle) {
            tab.journal.note(&mut tab.document);
        }
    }

    fn input(&mut self, event: WorkbenchInputEvent) -> bool {
        let tool = self.tool.clone();
        let used = self
            .with(|bench, ctx| bench.on_input(&event, tool.as_deref(), ctx).consumed)
            .unwrap_or(false);
        self.settle();
        used
    }

    /// Make `tool` the active one, or none, as a toolbar click does.
    pub fn tool(&mut self, tool: Option<&str>) -> bool {
        self.tool = tool.map(str::to_string);
        self.input(WorkbenchInputEvent::ToolActivated)
    }

    pub fn press(&mut self, at: (f32, f32)) -> bool {
        self.held.push(MouseButton::Left);
        self.input(WorkbenchInputEvent::MousePress {
            button: MouseButton::Left,
            viewport_pos: at,
        })
    }

    pub fn move_to(&mut self, at: (f32, f32)) -> bool {
        self.input(WorkbenchInputEvent::MouseMove { viewport_pos: at })
    }

    pub fn release(&mut self, at: (f32, f32)) -> bool {
        self.held.retain(|b| *b != MouseButton::Left);
        self.input(WorkbenchInputEvent::MouseRelease {
            button: MouseButton::Left,
            viewport_pos: at,
        })
    }

    /// Press at `from`, move to `to` in `steps` and let go there.
    pub fn drag(&mut self, from: (f32, f32), to: (f32, f32), steps: u32) {
        self.press(from);
        for i in 1..=steps.max(1) {
            let t = i as f32 / steps.max(1) as f32;
            self.move_to((from.0 + t * (to.0 - from.0), from.1 + t * (to.1 - from.1)));
        }
        self.release(to);
    }

    /// A key pressed and let go, while no card is open: with one, Enter
    /// and Escape are its OK and Cancel ([`ok`], [`cancel`]).
    ///
    /// [`ok`]: Self::ok
    /// [`cancel`]: Self::cancel
    pub fn key(&mut self, key: KeyCode) -> bool {
        let used = self.input(WorkbenchInputEvent::KeyPress { key });
        self.input(WorkbenchInputEvent::KeyRelease { key });
        used
    }

    /// The window loses focus: held buttons are let go.
    pub fn focus_lost(&mut self) {
        self.with(|bench, ctx| bench.cancel_pointer_gesture(ctx));
        self.held.clear();
        self.settle();
    }

    /// Type `value` into the card's text field `id`.
    pub fn type_text(&mut self, id: &str, value: &str) {
        let event = PanelEvent::Text {
            id: id.into(),
            value: value.into(),
        };
        self.card_event(event);
    }

    /// Any event of the card's, as its widgets send them; one naming a
    /// card that is not on screen is the package's to refuse.
    pub fn card_event(&mut self, event: PanelEvent) {
        self.with(|bench, ctx| bench.card_event(ctx, &event));
        self.settle();
    }

    /// The card's OK; `false` when there is no card or OK is held back.
    pub fn ok(&mut self) -> bool {
        self.close_card(true)
    }

    /// The card's Cancel: what was written while it was open is taken
    /// back. `false` when there is no card.
    pub fn cancel(&mut self) -> bool {
        self.close_card(false)
    }

    fn close_card(&mut self, accept: bool) -> bool {
        let Some(card) = self.card() else {
            return false;
        };
        if accept && card.refusal.is_some() {
            return false;
        }
        let event = PanelEvent::CardClose {
            id: card.id,
            accept,
        };
        self.with(|bench, ctx| bench.card_event(ctx, &event));
        let tab = self.tab();
        tab.card = None;
        let written = tab.journal.note(&mut tab.document);
        if !accept && let Some(entry) = written {
            tab.journal.undo_entry(&mut tab.document, entry);
        }
        self.settle();
        true
    }

    /// Undo the newest step; its label.
    pub fn undo(&mut self) -> Option<String> {
        let tab = self.tab();
        tab.journal.note(&mut tab.document);
        tab.journal.undo(&mut tab.document)
    }

    pub fn can_undo(&self) -> bool {
        self.tabs[self.on_screen]
            .as_ref()
            .is_some_and(|t| t.journal.can_undo())
    }

    /// Run one of the package's commands in the tab on screen, one step.
    pub fn run(&mut self, command: &str, args: serde_json::Value) -> CommandResult {
        let args: CommandArgs = args.as_object().cloned().unwrap_or_default();
        let result = self
            .with(|bench, ctx| bench.run_command(command, &args, ctx))
            .unwrap_or_else(|| Err(core_document::CommandError::Unknown(command.into())));
        let tab = self.tab();
        tab.journal.note(&mut tab.document);
        result
    }

    /// Open a tab with an empty document beside the others and bring it
    /// on screen; its number.
    pub fn open_tab(&mut self) -> usize {
        self.tabs
            .push(Some(Tab::new(&format!("tab {}", self.tabs.len()))));
        let index = self.tabs.len() - 1;
        self.switch_tab(index);
        index
    }

    /// Bring tab `index` on screen, the package suspended for the one
    /// leaving and resumed with what the one coming kept.
    pub fn switch_tab(&mut self, index: usize) {
        if index == self.on_screen || self.tabs.get(index).is_none_or(Option::is_none) {
            return;
        }
        let kept = self.bench.as_mut().and_then(|b| b.suspend_session());
        self.tab().kept = kept;
        self.held.clear();
        self.on_screen = index;
        let kept = self.tab().kept.take();
        if let Some(bench) = self.bench.as_mut() {
            bench.resume_session(kept);
        }
        self.settle();
    }

    /// Close tab `index`, not the one on screen: what the package kept of
    /// it is dropped, its jobs stopped.
    pub fn close_tab(&mut self, index: usize) {
        assert_ne!(index, self.on_screen, "close a tab off screen");
        self.tabs[index] = None;
    }

    /// Give the package the frames the app gives it until its jobs are
    /// told, or `timeout` passes; whether it is idle.
    pub fn wait_for_jobs(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let busy = self.with(|bench, ctx| {
                bench.on_frame(0.016, ctx);
                bench.jobs_pending()
            });
            if busy != Some(true) {
                self.settle();
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
