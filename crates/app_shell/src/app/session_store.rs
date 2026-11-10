//! What one run of the app leaves for the next: each document's view, by
//! its file (`views.json`), and the session's open documents, window and
//! panels (`session.json`), so a start looks as the last run ended.
//!
//! They live in the application's own folder beside `chats.json`, never
//! in the document: a view change is not an edit, and two people on one
//! document through its server each keep their own view.
//!
//! They are kept while the app runs, a moment after they last changed,
//! and again on the way out, so a run that ends without quitting (killed
//! from its terminal, crashed) still leaves them.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::PrintCadApp;
use crate::app::session::DocumentSession;
use crate::camera::KeptView;
use crate::log_panel as app_log;
use crate::ui::PanelLayout;

/// How many documents' views are kept: the latest looked at.
pub(crate) const VIEWS_KEPT: usize = 50;

const VIEWS_FILE: &str = "views.json";
const SESSION_FILE: &str = "session.json";

/// A document's view, kept with its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct FileView {
    file: String,
    view: KeptView,
}

/// Every kept view, the latest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Views {
    #[serde(default)]
    views: Vec<FileView>,
}

impl Views {
    fn get(&self, key: &str) -> Option<&KeptView> {
        self.views.iter().find(|v| v.file == key).map(|v| &v.view)
    }

    /// Put `view` first for `key`, dropping the oldest past the bound.
    fn put(&mut self, key: String, view: KeptView) {
        self.views.retain(|v| v.file != key);
        self.views.insert(0, FileView { file: key, view });
        self.views.truncate(VIEWS_KEPT);
    }
}

/// Where the window stood, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub(crate) struct WindowPlace {
    /// The size inside the frame, unmaximized.
    pub width: u32,
    pub height: u32,
    /// The frame's top left; `None` where the system does not tell it.
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub maximized: bool,
}

/// The session as the last run left it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct KeptSession {
    /// The files of the open tabs, in the strip's order.
    pub tabs: Vec<PathBuf>,
    /// Which of them was on screen.
    pub active: usize,
    pub window: Option<WindowPlace>,
    pub panels: Option<PanelLayout>,
}

impl KeptSession {
    /// The tabs to open again: the files still there, in order, with the
    /// one to bring on screen among them (the nearest one before it when
    /// the active file is gone).
    pub(crate) fn tabs_to_reopen(&self, exists: impl Fn(&Path) -> bool) -> (Vec<PathBuf>, usize) {
        let mut active = 0;
        let mut kept = Vec::new();
        for (index, file) in self.tabs.iter().enumerate() {
            if !exists(file) || kept.contains(file) {
                continue;
            }
            if index <= self.active {
                active = kept.len();
            }
            kept.push(file.clone());
        }
        (kept, active)
    }
}

/// How long the session stands still before it is kept: a view is kept
/// once the camera rests, not at every step of an orbit.
const KEEP_AFTER: Duration = Duration::from_secs(1);

/// The session as it stands, with each tab's view by its file.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Standing {
    session: KeptSession,
    views: Vec<(PathBuf, KeptView)>,
}

/// What this run saw of its session and what it kept.
#[derive(Debug, Default)]
pub(crate) struct SessionKeeping {
    /// The session as last seen, and since when it has stood so.
    seen: Option<(Standing, Instant)>,
    kept: Option<Standing>,
}

impl SessionKeeping {
    /// Look at the session as it stands `at`: what to keep, once it has
    /// stood so for [`KEEP_AFTER`] and is not what is kept already.
    fn see(&mut self, now: Standing, at: Instant) -> Option<Standing> {
        match &self.seen {
            Some((seen, since)) if *seen == now => (at.duration_since(*since) >= KEEP_AFTER
                && self.kept.as_ref() != Some(&now))
            .then_some(now),
            _ => {
                self.seen = Some((now, at));
                None
            }
        }
    }

    /// When the session seen will be due to be kept.
    fn due(&self) -> Option<Instant> {
        let (seen, since) = self.seen.as_ref()?;
        (self.kept.as_ref() != Some(seen)).then(|| *since + KEEP_AFTER)
    }
}

/// What the stores are keyed by: the file as an absolute path.
fn key(file: &Path) -> String {
    std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .display()
        .to_string()
}

fn store_path(name: &str) -> Option<PathBuf> {
    let path = settings::config_path(name)?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    Some(path)
}

use settings::json_file::{keep as write, load};

/// The view kept with `file`.
pub(crate) fn view_of(file: &Path) -> Option<KeptView> {
    view_in(&store_path(VIEWS_FILE)?, file)
}

/// Keep each file's view, in place of what it had.
pub(crate) fn keep_views(views: Vec<(PathBuf, KeptView)>) {
    if views.is_empty() {
        return;
    }
    if let Some(path) = store_path(VIEWS_FILE) {
        keep_views_in(&path, views);
    }
}

fn view_in(store: &Path, file: &Path) -> Option<KeptView> {
    match load::<Views>(store) {
        Ok(all) => all.get(&key(file)).cloned(),
        Err(err) => {
            app_log::warn(format!("The document's view is not restored: {err}"));
            None
        }
    }
}

/// Keep `views` in `store`. A store that does not read is left as it is,
/// rather than written over with these views alone.
fn keep_views_in(store: &Path, views: Vec<(PathBuf, KeptView)>) {
    let mut all = match load::<Views>(store) {
        Ok(all) => all,
        Err(err) => {
            app_log::warn(format!("The documents' views are not kept: {err}"));
            return;
        }
    };
    let before = all.clone();
    for (file, view) in views {
        all.put(key(&file), view);
    }
    if all != before
        && let Err(err) = write(store, &all)
    {
        app_log::warn(format!("Could not keep the documents' views: {err}"));
    }
}

/// The session the last run left; empty when there is none.
pub(crate) fn last_session() -> KeptSession {
    let Some(path) = store_path(SESSION_FILE) else {
        return KeptSession::default();
    };
    load(&path).unwrap_or_else(|err| {
        app_log::warn(format!("The last session is not restored: {err}"));
        KeptSession::default()
    })
}

/// Keep `session` for the next start.
pub(crate) fn keep_session(session: &KeptSession) {
    if let Some(path) = store_path(SESSION_FILE)
        && let Err(err) = write(&path, session)
    {
        app_log::warn(format!("Could not keep the session: {err}"));
    }
}

impl DocumentSession {
    /// The view to keep with this tab's file: none while the opened
    /// document still waits to be framed, which is no view the user chose.
    pub(crate) fn view_to_keep(&self) -> Option<KeptView> {
        (!self.fit_on_layout).then(|| KeptView {
            side: self
                .side_camera
                .as_ref()
                .map(|camera| Box::new(camera.kept_view())),
            ..self.camera.kept_view()
        })
    }
}

impl PrintCadApp {
    /// Put the view kept with `file` on the tab's camera. Whether there
    /// was one to put back.
    pub(crate) fn restore_kept_view(&mut self, file: &Path) -> bool {
        let Some(view) = view_of(file) else {
            return false;
        };
        let settings = &self.user_settings.camera;
        let restored = self.session.camera.restore_view(&view, settings);
        self.session.side_camera = view.side.and_then(|side| {
            let mut camera = self.session.camera.detached(settings);
            camera.restore_view(&side, settings).then_some(camera)
        });
        restored
    }

    /// The session as it ends, for the next start. Runs on the way out,
    /// once every save has finished, so a Save as taken while quitting
    /// has its file.
    pub(crate) fn keep_session_for_next_run(&mut self) {
        let now = self.session_now();
        self.keep(now);
    }

    /// Keep the session once it has stood still a moment, while no tab is
    /// opening, saving or building, which would keep it half arrived.
    pub(crate) fn drive_session_keeping(&mut self) {
        if self.gfx.is_none() || self.any_tab_busy() {
            return;
        }
        let now = self.session_now();
        if let Some(due) = self.session_keeping.see(now, Instant::now()) {
            self.keep(due);
        }
    }

    /// Keep the benches' settings when a bench changed its own outside
    /// Preferences (a switch on its task panel, such as the sketcher's grid
    /// and snap), as a Preferences commit keeps them: in
    /// `UserSettings.workbenches`, written to the settings file. While
    /// Preferences is open its commit does this.
    pub(crate) fn drive_bench_settings_keeping(&mut self) {
        if self
            .gfx
            .as_ref()
            .is_some_and(|gfx| gfx.ui_layer.preferences_open())
        {
            return;
        }
        let now = self.registry.collect_settings();
        if !bench_settings_moved(&self.user_settings.workbenches, &now) {
            return;
        }
        self.user_settings.workbenches.extend(now);
        if let Err(err) = self.settings_store.save(&self.user_settings) {
            app_log::warn(format!("Failed to save settings: {err}"));
        }
    }

    /// When the loop must wake to keep the session, idle or not.
    pub(crate) fn next_session_keeping(&self) -> Option<Instant> {
        self.session_keeping.due()
    }

    /// Write `now` to the stores. A run a bench hook opened leaves the
    /// user's session as it was, as it leaves it unopened at start.
    fn keep(&mut self, now: Standing) {
        if crate::launch_screen() == crate::ui::Screen::Start {
            keep_views(now.views.clone());
            keep_session(&now.session);
        }
        self.last_session = now.session.clone();
        self.session_keeping.kept = Some(now);
    }

    /// Each tab's view with its file, the files open in the strip, where
    /// the window stands and how the panels are.
    fn session_now(&self) -> Standing {
        let tabs: Vec<(Option<PathBuf>, Option<KeptView>)> = self
            .tabs
            .iter()
            .map(|slot| {
                let session = slot.parked.as_ref().unwrap_or(&self.session);
                (session.current_file.clone(), session.view_to_keep())
            })
            .collect();
        let views = tabs
            .iter()
            .filter_map(|(file, view)| Some((file.clone()?, view.clone()?)))
            .collect();
        let files: Vec<PathBuf> = tabs.iter().filter_map(|(file, _)| file.clone()).collect();
        // The active tab among those with a file: the one itself, or the
        // nearest before it when it has none.
        let active = tabs[..=self.active_tab.min(tabs.len().saturating_sub(1))]
            .iter()
            .filter(|(file, _)| file.is_some())
            .count()
            .saturating_sub(1);
        let window = self
            .gfx
            .as_ref()
            .map(|gfx| window_place(&gfx.window, self.last_session.window));
        let panels = self.gfx.as_ref().map(|gfx| gfx.ui_layer.panel_layout());
        Standing {
            session: KeptSession {
                tabs: files,
                active,
                window: window.or(self.last_session.window),
                panels: panels.or_else(|| self.last_session.panels.clone()),
            },
            views,
        }
    }

    /// At start, the documents to open: those named on the command line,
    /// else the ones open when the app last closed, as Preferences allow.
    /// A bench hook that opens something leaves both alone.
    pub(crate) fn reopen_at_start(&mut self) {
        if !self.launch_files.is_empty() {
            let files = std::mem::take(&mut self.launch_files);
            self.open_dropped(files);
            return;
        }
        if !self.user_settings.session.reopen_documents {
            return;
        }
        let (files, active) = self.last_session.tabs_to_reopen(|path| path.is_file());
        if files.is_empty() {
            return;
        }
        for file in files {
            self.open_document_at(file);
        }
        self.switch_tab(active);
    }
}

/// Where `window` stands now. A maximized window keeps the size it had
/// before, from `before`, to come back to when it is unmaximized.
fn window_place(window: &winit::window::Window, before: Option<WindowPlace>) -> WindowPlace {
    let size = window.inner_size();
    let maximized = window.is_maximized();
    let (width, height) = match before {
        Some(place) if maximized => (place.width, place.height),
        _ => (size.width, size.height),
    };
    let position = if maximized {
        before.and_then(|place| place.x.zip(place.y))
    } else {
        window.outer_position().ok().map(|p| (p.x, p.y))
    };
    WindowPlace {
        width,
        height,
        x: position.map(|p| p.0),
        y: position.map(|p| p.1),
        maximized,
    }
}

/// `attributes` with the window placed where the last run left it: its
/// size, its position when it still lies on a screen, maximized or not.
pub(crate) fn placed_window(
    attributes: winit::window::WindowAttributes,
    place: Option<WindowPlace>,
    event_loop: &winit::event_loop::ActiveEventLoop,
) -> winit::window::WindowAttributes {
    let Some(place) = place else {
        return attributes;
    };
    let mut attributes = attributes;
    if place.width >= MIN_WINDOW.0 && place.height >= MIN_WINDOW.1 {
        attributes =
            attributes.with_inner_size(winit::dpi::PhysicalSize::new(place.width, place.height));
    }
    if let Some((x, y)) = place.x.zip(place.y) {
        let on_a_screen = event_loop.available_monitors().any(|monitor| {
            let (at, size) = (monitor.position(), monitor.size());
            let (left, top) = (i64::from(at.x), i64::from(at.y));
            let (right, bottom) = (left + i64::from(size.width), top + i64::from(size.height));
            // The title bar's corner, a little in, must be somewhere seen.
            let (px, py) = (i64::from(x) + 40, i64::from(y) + 20);
            (left..right).contains(&px) && (top..bottom).contains(&py)
        });
        if on_a_screen {
            attributes = attributes.with_position(winit::dpi::PhysicalPosition::new(x, y));
        }
    }
    attributes.with_maximized(place.maximized)
}

/// The least window size put back; a smaller one is taken as a mistake.
const MIN_WINDOW: (u32, u32) = (320, 240);

/// Whether any bench now holds settings other than the ones kept. A bench
/// missing from `now` (a package turned off) keeps what was kept for it.
fn bench_settings_moved(
    kept: &std::collections::HashMap<String, serde_json::Value>,
    now: &std::collections::HashMap<String, serde_json::Value>,
) -> bool {
    now.iter().any(|(id, value)| kept.get(id) != Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bench_setting_changed_on_its_panel_is_kept() {
        let kept: std::collections::HashMap<String, serde_json::Value> = [
            ("wb.a".to_owned(), serde_json::json!({"grid_on": false})),
            ("pkg.off".to_owned(), serde_json::json!({"x": 1})),
        ]
        .into();
        let same: std::collections::HashMap<String, serde_json::Value> =
            [("wb.a".to_owned(), serde_json::json!({"grid_on": false}))].into();
        let toggled: std::collections::HashMap<String, serde_json::Value> =
            [("wb.a".to_owned(), serde_json::json!({"grid_on": true}))].into();
        assert!(!bench_settings_moved(&kept, &same));
        assert!(bench_settings_moved(&kept, &toggled));
        assert!(bench_settings_moved(
            &std::collections::HashMap::new(),
            &same
        ));
    }

    fn view(distance: f64) -> KeptView {
        KeptView {
            target: [1.0, 2.0, 3.0],
            orientation: [0.0, 0.0, 0.0, 1.0],
            distance,
            projection: settings::ProjectionMode::Orthographic,
            field_of_view_deg: 40.0,
            ortho_height: 120.0,
            axis_preset: axes::AxisPreset::default(),
            side: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("printcad-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_file_keeps_its_own_view() {
        let dir = scratch("views");
        let store = dir.join(VIEWS_FILE);
        let (a, b) = (dir.join("a.prtcad"), dir.join("b.prtcad"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        keep_views_in(
            &store,
            vec![(a.clone(), view(10.0)), (b.clone(), view(20.0))],
        );
        assert_eq!(view_in(&store, &a), Some(view(10.0)));
        // The same file by another path is the same document.
        assert_eq!(
            view_in(&store, &dir.join(".").join("b.prtcad")),
            Some(view(20.0))
        );
        keep_views_in(&store, vec![(a.clone(), view(30.0))]);
        assert_eq!(view_in(&store, &a), Some(view(30.0)));
        assert_eq!(view_in(&store, &dir.join("c.prtcad")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The store keeps the latest views looked at, and no more.
    #[test]
    fn the_views_kept_are_bounded_and_the_latest_stay() {
        let mut all = Views::default();
        for n in 0..VIEWS_KEPT + 10 {
            all.put(format!("/d/{n}.prtcad"), view(n as f64 + 1.0));
        }
        assert_eq!(all.views.len(), VIEWS_KEPT);
        let last = VIEWS_KEPT + 9;
        assert_eq!(all.views[0].file, format!("/d/{last}.prtcad"));
        assert!(all.get("/d/0.prtcad").is_none(), "the oldest went");
        // Looking at a kept one again brings it to the front, once.
        all.put("/d/20.prtcad".into(), view(99.0));
        assert_eq!(all.views.len(), VIEWS_KEPT);
        assert_eq!(all.views[0].file, "/d/20.prtcad");
        assert_eq!(
            all.views
                .iter()
                .filter(|v| v.file == "/d/20.prtcad")
                .count(),
            1
        );
        let again: Views = serde_json::from_str(&serde_json::to_string(&all).unwrap()).unwrap();
        assert_eq!(again, all);
    }

    /// A store that does not read is never written over: every other
    /// document's view stays for whoever mends it.
    #[test]
    fn a_views_store_that_does_not_read_is_left_as_it_is() {
        let dir = scratch("views-bad");
        let store = dir.join(VIEWS_FILE);
        std::fs::write(&store, "{ not json").unwrap();
        keep_views_in(&store, vec![(dir.join("a.prtcad"), view(1.0))]);
        assert_eq!(std::fs::read_to_string(&store).unwrap(), "{ not json");
        assert_eq!(view_in(&store, &dir.join("a.prtcad")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_session_reads_back_as_it_was_written() {
        let mut panels = PanelLayout {
            console_open: true,
            tree_share: Some(0.4),
            ..PanelLayout::default()
        };
        panels.sizes.insert("combo_view".into(), 310.0);
        let session = KeptSession {
            tabs: vec!["/d/a.prtcad".into(), "/d/b.prtcad".into()],
            active: 1,
            window: Some(WindowPlace {
                width: 1600,
                height: 900,
                x: Some(40),
                y: None,
                maximized: true,
            }),
            panels: Some(panels),
        };
        let text = serde_json::to_string(&session).unwrap();
        assert_eq!(serde_json::from_str::<KeptSession>(&text).unwrap(), session);
        // A file from an older or newer build reads what it can.
        let sparse: KeptSession = serde_json::from_str(r#"{"tabs": ["/d/a.prtcad"]}"#).unwrap();
        assert_eq!(sparse.tabs.len(), 1);
        assert_eq!(sparse.window, None);
    }

    /// The session is kept once it has stood still, once for each way it
    /// stands, and the loop is told when to wake for it.
    #[test]
    fn the_session_is_kept_once_it_rests() {
        let standing = |distance| Standing {
            session: KeptSession {
                tabs: vec!["/d/a.prtcad".into()],
                ..KeptSession::default()
            },
            views: vec![("/d/a.prtcad".into(), view(distance))],
        };
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut keeping = SessionKeeping::default();
        assert_eq!(keeping.due(), None, "nothing seen, nothing due");
        // An orbit: every frame a new view, none kept.
        for (ms, distance) in [(0, 10.0), (400, 11.0), (800, 12.0)] {
            assert_eq!(keeping.see(standing(distance), at(ms)), None);
        }
        assert_eq!(keeping.due(), Some(at(800) + KEEP_AFTER));
        assert_eq!(keeping.see(standing(12.0), at(1500)), None, "too soon");
        let kept = keeping.see(standing(12.0), at(1800));
        assert_eq!(kept, Some(standing(12.0)));
        keeping.kept = kept;
        assert_eq!(keeping.due(), None);
        assert_eq!(keeping.see(standing(12.0), at(9000)), None, "kept once");
        // Back where it was kept: nothing new to keep.
        assert_eq!(keeping.see(standing(13.0), at(9100)), None);
        assert_eq!(keeping.see(standing(12.0), at(9200)), None);
        assert_eq!(keeping.see(standing(12.0), at(9200) + KEEP_AFTER), None);
    }

    /// Files that went since are left out of the reopening, and the tab
    /// that comes on screen is the one that was, or the one before it.
    #[test]
    fn reopening_skips_files_that_are_gone() {
        let session = KeptSession {
            tabs: ["/a", "/gone1", "/b", "/gone2", "/c", "/b"]
                .map(PathBuf::from)
                .to_vec(),
            active: 4,
            ..KeptSession::default()
        };
        let exists = |p: &Path| !p.to_string_lossy().contains("gone");
        let (tabs, active) = session.tabs_to_reopen(exists);
        assert_eq!(tabs, ["/a", "/b", "/c"].map(PathBuf::from));
        assert_eq!(active, 2);
        let gone_active = KeptSession {
            active: 3,
            ..session.clone()
        };
        assert_eq!(gone_active.tabs_to_reopen(exists).1, 1, "the one before it");
        let nothing = KeptSession {
            tabs: vec!["/gone".into()],
            ..KeptSession::default()
        };
        assert_eq!(nothing.tabs_to_reopen(exists), (Vec::new(), 0));
    }
}
