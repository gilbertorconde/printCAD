//! Autosave and recovery: every few minutes a copy of each document edited
//! since it was saved goes to the recovery folder, beside a note of what it
//! is. A tab saved or closed, and the application quitting, take their
//! copies away, so what the folder holds at start is what a crash left:
//! the start page offers it back.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a copy is, kept beside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Note {
    name: String,
    /// The document's own file, when it had one.
    file: Option<PathBuf>,
    /// The process that wrote it: while it runs, the copy is its own.
    pid: u32,
    /// When it was written, in milliseconds since the Unix epoch.
    saved_ms: u64,
}

/// A copy a crash left, which the start page offers back.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Recoverable {
    pub copy: PathBuf,
    pub name: String,
    pub file: Option<PathBuf>,
    pub saved_ms: u64,
}

/// Where copies go: the app's data folder, else the temp folder.
fn dir() -> PathBuf {
    settings::recovery_dir().unwrap_or_else(|| std::env::temp_dir().join("printcad-recovery"))
}

fn copy_of(tab: Uuid) -> PathBuf {
    dir().join(format!("{tab}.prtcad"))
}

fn note_of(copy: &Path) -> PathBuf {
    copy.with_extension("json")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Keep `bytes` as tab `tab`'s copy: written beside it and moved over it,
/// so a crash mid-write leaves the last whole copy.
pub(crate) fn keep(
    tab: Uuid,
    name: &str,
    file: Option<&Path>,
    bytes: &[u8],
) -> std::io::Result<()> {
    let copy = copy_of(tab);
    std::fs::create_dir_all(dir())?;
    let partial = copy.with_extension("partial");
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, &copy)?;
    let note = Note {
        name: name.to_string(),
        file: file.map(Path::to_path_buf),
        pid: std::process::id(),
        saved_ms: now_ms(),
    };
    crate::platform::write(&note_of(&copy), &serde_json::to_vec_pretty(&note)?)
}

/// Take tab `tab`'s copy away: it was saved, closed, or the app quits.
pub(crate) fn forget(tab: Uuid) {
    remove(&copy_of(tab));
}

/// Take a copy and its note away.
pub(crate) fn remove(copy: &Path) {
    crate::platform::remove(copy);
    crate::platform::remove(&note_of(copy));
}

/// Whether process `pid` still runs, as far as the system says.
fn running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// The copies left by processes no longer running, the latest first.
pub(crate) fn left_behind() -> Vec<Recoverable> {
    left_behind_in(&dir())
}

fn left_behind_in(dir: &Path) -> Vec<Recoverable> {
    let mut found: Vec<Recoverable> = crate::platform::list(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|note_path| {
            let note: Note =
                serde_json::from_slice(&crate::platform::read(&note_path).ok()?).ok()?;
            let copy = note_path.with_extension("prtcad");
            (crate::platform::exists(&copy) && !running(note.pid)).then_some(Recoverable {
                copy,
                name: note.name,
                file: note.file,
                saved_ms: note.saved_ms,
            })
        })
        .collect();
    found.sort_by_key(|r| std::cmp::Reverse(r.saved_ms));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A copy whose writer is gone is left behind; one this process wrote
    /// is its own, still in use.
    #[test]
    fn copies_of_a_process_gone_are_left_behind() {
        let dir = std::env::temp_dir().join(format!("printcad-recovery-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, pid: u32, saved_ms: u64| {
            let copy = dir.join(format!("{name}.prtcad"));
            std::fs::write(&copy, b"copy").unwrap();
            let note = Note {
                name: name.to_string(),
                file: None,
                pid,
                saved_ms,
            };
            std::fs::write(note_of(&copy), serde_json::to_vec(&note).unwrap()).unwrap();
        };
        // No process has an id this high on any system in use.
        write("older", u32::MAX - 1, 1);
        write("newer", u32::MAX - 1, 2);
        write("mine", std::process::id(), 3);
        let names: Vec<String> = left_behind_in(&dir).into_iter().map(|r| r.name).collect();
        assert_eq!(names, ["newer", "older"]);
        remove(&dir.join("newer.prtcad"));
        assert_eq!(left_behind_in(&dir).len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
