//! Crash reports: a panic leaves a folder in the data folder's
//! `crash-reports/` with what went wrong and, for each tab, what replays
//! it: the document as it was when the trail began (its file, a copy, or
//! nothing for an empty one) and every command run on it since, as a
//! script.
//!
//! Opening `1-start.prtcad` and running `1-calls.lua` on it (Scripts ›
//! Run script…) replays tab 1, as does `printcad --script 1-calls.lua
//! --open 1-start.prtcad` when its calls need no window; the report's
//! `README.txt` gives each tab's command.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use core_document::{Document, Recorded};
use uuid::Uuid;

/// What a tab did since its document was last as its file is.
struct Trail {
    name: String,
    /// What the calls start from; `None` for an empty document.
    start: Option<Start>,
    recorder: scripting::Recorder,
    /// The document's id and edit count when the trail began.
    at: (Uuid, u64),
}

/// Where a trail begins: a clean document's file, or a copy of one that
/// came to the tab with content no call made (a start card's).
enum Start {
    File(PathBuf),
    Bytes(Vec<u8>),
}

static TRAILS: LazyLock<Mutex<HashMap<Uuid, Trail>>> = LazyLock::new(Mutex::default);

/// At most this many reports a run: a panic caught and met again would
/// otherwise write one every frame.
const REPORTS_A_RUN: usize = 3;

fn trails() -> std::sync::MutexGuard<'static, HashMap<Uuid, Trail>> {
    TRAILS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Bring tab `tab`'s trail up to its document. The trail starts over
/// when the tab shows another document (opened, new) and when the one it
/// shows is clean again, unlike when the trail began (saved, undone back):
/// a clean document is its file. A call whose change has not landed yet
/// (an import) leaves the edit count as it was, and the trail with it.
pub(crate) fn sync(tab: Uuid, document: &mut Document, file: Option<&Path>) {
    let clean = !document.metadata().dirty();
    let at = (document.id(), document.mutation_seq());
    let name = document.name().to_string();
    let mut trails = trails();
    let same_file = |trail: &Trail| match &trail.start {
        Some(Start::File(start)) => Some(start.as_path()) == file,
        _ => file.is_none(),
    };
    match trails.get_mut(&tab) {
        Some(trail) if trail.at.0 == at.0 && (!clean || (trail.at == at && same_file(trail))) => {
            trail.name = name;
        }
        _ => {
            let empty = document.bodies().is_empty()
                && document.feature_tree().all_nodes().next().is_none();
            let start = match file {
                Some(file) if clean => Some(Start::File(file.to_path_buf())),
                _ if empty => None,
                _ => document
                    .save_to_bytes(core_document::Compression::Zstd)
                    .ok()
                    .map(Start::Bytes),
            };
            let trail = Trail {
                name,
                start,
                recorder: scripting::Recorder::default(),
                at,
            };
            trails.insert(tab, trail);
        }
    }
}

/// Keep `calls`, run on tab `tab`, in its trail.
pub(crate) fn note(tab: Uuid, calls: &[Recorded]) {
    if let Some(trail) = trails().get_mut(&tab) {
        for call in calls {
            trail.recorder.push(call);
        }
    }
}

/// Drop tab `tab`'s trail: the tab closed, or its document took content
/// no call made, so the next sync starts one from the document as it is.
pub(crate) fn forget(tab: Uuid) {
    trails().remove(&tab);
}

/// Report every panic, after the hook before this one has had its say.
pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        static WRITTEN: AtomicUsize = AtomicUsize::new(0);
        if WRITTEN.fetch_add(1, Ordering::Relaxed) >= REPORTS_A_RUN {
            return;
        }
        let Some(root) = settings::crash_reports_dir() else {
            return;
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        let dir = root.join(format!("crash-{stamp}"));
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".into());
        let place = info.location().map_or_else(String::new, |l| l.to_string());
        let what = format!(
            "thread `{}` panicked at {place}:\n{message}\n\nbacktrace:\n{}",
            std::thread::current().name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture()
        );
        match write_report(&dir, &what) {
            Ok(()) => eprintln!("printCAD: a crash report is in {}", dir.display()),
            Err(err) => eprintln!("printCAD: the crash report could not be written: {err}"),
        }
    }));
}

/// Write the report: `report.txt` with `what` and the version, and each
/// tab's start file and calls, with a README saying how to replay them.
fn write_report(dir: &Path, what: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut readme = String::from(
        "printCAD crash report. report.txt is what went wrong.\n\
         Each tab's work replays by opening its start file in printCAD and\n\
         running its calls (Scripts > Run script...), or with the command\n\
         beside it when the calls need no window.\n\n",
    );
    for (n, trail) in trails().values().enumerate() {
        let n = n + 1;
        let calls = format!("{n}-calls.lua");
        let mut replay = format!("printcad --script {calls}");
        let copy = format!("{n}-start.prtcad");
        let copied = match &trail.start {
            Some(Start::File(start)) => std::fs::copy(start, dir.join(&copy)).map(drop),
            Some(Start::Bytes(bytes)) => std::fs::write(dir.join(&copy), bytes),
            None => Err(std::io::ErrorKind::NotFound.into()),
        };
        if copied.is_ok() {
            let _ = write!(replay, " --open {copy}");
        }
        let about = format!("What was done to \"{}\" before the crash", trail.name);
        std::fs::write(dir.join(&calls), trail.recorder.script(&about))?;
        let _ = writeln!(readme, "{} ({n}): {replay}", trail.name);
    }
    std::fs::write(dir.join("README.txt"), readme)?;
    std::fs::write(
        dir.join("report.txt"),
        format!("printCAD {}\n\n{what}\n", env!("CARGO_PKG_VERSION")),
    )
}

#[cfg(test)]
mod tests {
    /// A tab's trail written as a report replays to the document it made:
    /// its calls run on the start the report keeps beside them (here a
    /// new document a start card filled, with edits no call made).
    #[test]
    fn a_crash_report_replays_what_its_tab_did() {
        let dir = std::env::temp_dir().join(format!("printcad-crash-{}", uuid::Uuid::new_v4()));
        let tab = uuid::Uuid::new_v4();
        let mut start = core_document::Document::new("Plate");
        start.create_body(Some("Base".into()));
        super::sync(tab, &mut start, None);
        let made = uuid::Uuid::new_v4().to_string();
        let call = |id: &str, args: serde_json::Value, result| core_document::Recorded {
            id: id.into(),
            args: args.as_object().unwrap().clone(),
            result,
        };
        let calls = [
            call(
                "doc.new_body",
                serde_json::json!({"name": "Lid"}),
                made.clone().into(),
            ),
            call(
                "doc.rename",
                serde_json::json!({"id": made, "name": "Top"}),
                "".into(),
            ),
        ];
        super::note(tab, &calls);
        super::write_report(&dir, "test").unwrap();
        super::forget(tab);
        let readme = std::fs::read_to_string(dir.join("README.txt")).unwrap();
        let line = readme.lines().find(|l| l.starts_with("Plate")).unwrap();
        let words: Vec<&str> = line.split_whitespace().collect();
        let (script, open) = (words[4], words[6]);
        let source = std::fs::read_to_string(dir.join(script)).unwrap();
        let document = core_document::Document::load_from_file(&dir.join(open)).unwrap();
        let mut registry = core_document::DocumentService::default();
        let (replayed, answer) =
            crate::headless::run_in_document(&mut registry, document, &source, script);
        answer.unwrap();
        let names: Vec<_> = replayed.bodies().iter().map(|b| b.name.clone()).collect();
        assert_eq!(names, ["Base", "Top"], "{source}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
