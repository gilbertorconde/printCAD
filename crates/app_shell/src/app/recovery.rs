//! Autosave and recovery: every few minutes a copy of each document edited
//! since it was saved goes to the recovery folder, beside a note of what it
//! is. A tab saved or closed, and the application quitting, take their
//! copies away, so what the folder holds at start is what a crash left,
//! or an exit forced by a window that cannot draw: the start page offers
//! it back. Beside them it offers the documents whose op logs end with
//! edits a session left unsaved (`doc_server::daemon::unsaved_logs`):
//! Recover opens the document and brings them back through its server.
//! An untitled tab's daemon keeps its log until the tab is saved or let
//! go, so a crash leaves it too (`doc_server::daemon::untitled_logs`):
//! Recover brings its edits into a new untitled tab through that tab's
//! server. The log holds every edit the daemon was sent, the tab's copy
//! only those up to the last autosave, so where both are left the log
//! stands for the two, and the copy goes with it.

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
    /// What names it: the copy, the document whose log holds the edits,
    /// or the socket of the untitled tab's daemon that kept them.
    pub copy: PathBuf,
    pub name: String,
    pub file: Option<PathBuf>,
    pub saved_ms: u64,
    pub source: Source,
}

/// Where the work a crash left is kept.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Source {
    /// An autosave copy. `untitled` is the socket its tab had while it had
    /// no file, which names the log its daemon kept.
    Copy { untitled: Option<PathBuf> },
    /// Unsaved edits the document's op log holds.
    DocumentLog,
    /// The log an untitled tab's daemon kept. `copy_too` is that tab's
    /// autosave copy, which goes with it.
    UntitledLog { copy_too: Option<PathBuf> },
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
    app_platform::write(&note_of(&copy), &serde_json::to_vec_pretty(&note)?)
}

/// Take tab `tab`'s copy away: it was saved, closed, or the app quits.
pub(crate) fn forget(tab: Uuid) {
    remove(&copy_of(tab));
}

/// Take a copy and its note away.
pub(crate) fn remove(copy: &Path) {
    app_platform::remove(copy);
    app_platform::remove(&note_of(copy));
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

/// The copies left by processes no longer running and the documents whose
/// logs hold unsaved edits, the latest first.
pub(crate) fn left_behind() -> Vec<Recoverable> {
    with_logs(
        left_behind_in(&dir()),
        doc_server::daemon::unsaved_logs(),
        doc_server::daemon::untitled_logs(),
    )
}

/// `copies` with the documents `logs` names and the untitled sessions'
/// logs `untitled`, each standing for its tab's copy, the latest first.
fn with_logs(
    mut copies: Vec<Recoverable>,
    logs: Vec<doc_server::daemon::UnsavedLog>,
    untitled: Vec<doc_server::daemon::UntitledLog>,
) -> Vec<Recoverable> {
    for log in untitled {
        let copy = copies
            .iter()
            .position(|copy| {
                matches!(&copy.source, Source::Copy { untitled: Some(socket) } if *socket == log.socket)
            })
            .map(|at| copies.remove(at));
        copies.push(Recoverable {
            name: copy
                .as_ref()
                .map_or_else(|| "Untitled".to_string(), |copy| copy.name.clone()),
            copy: log.socket,
            file: None,
            saved_ms: log.modified_ms,
            source: Source::UntitledLog {
                copy_too: copy.map(|copy| copy.copy),
            },
        });
    }
    copies.extend(logs.into_iter().map(|log| {
        Recoverable {
            name: log
                .document
                .file_name()
                .map(|name| {
                    crate::app::doc_io::document_name_from_file_name(&name.to_string_lossy())
                        .to_string()
                })
                .unwrap_or_default(),
            file: Some(log.document.clone()),
            copy: log.document,
            saved_ms: log.modified_ms,
            source: Source::DocumentLog,
        }
    }));
    copies.sort_by_key(|r| std::cmp::Reverse(r.saved_ms));
    copies
}

fn left_behind_in(dir: &Path) -> Vec<Recoverable> {
    let mut found: Vec<Recoverable> = app_platform::list(dir)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|note_path| {
            let note: Note = serde_json::from_slice(&app_platform::read(&note_path).ok()?).ok()?;
            let copy = note_path.with_extension("prtcad");
            let untitled = note
                .file
                .is_none()
                .then(|| copy.file_stem()?.to_str()?.parse::<Uuid>().ok())
                .flatten()
                .map(|tab| doc_server::socket_path_for_untitled_of(note.pid, tab));
            (app_platform::exists(&copy) && !running(note.pid)).then_some(Recoverable {
                copy,
                name: note.name,
                file: note.file,
                saved_ms: note.saved_ms,
                source: Source::Copy { untitled },
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

    /// A document whose op log ends with an edit, or with a mark naming
    /// unsaved edits, is offered beside the copies, in the order written;
    /// one whose log ends saved is not, nor one whose file is gone.
    #[test]
    fn documents_whose_logs_end_unsaved_are_offered_with_the_copies() {
        let dir = std::env::temp_dir().join(format!("printcad-oplog-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = |key: &str, document: &Path, last: serde_json::Value| {
            std::fs::write(document, b"file").unwrap();
            let note = dir.join(format!("{key}.oplog.document"));
            std::fs::write(&note, serde_json::to_vec(document).unwrap()).unwrap();
            let first = serde_json::json!({"op": {}, "batch": Uuid::new_v4()});
            std::fs::write(note.with_extension("jsonl"), format!("{first}\n{last}\n")).unwrap();
        };
        let edited = dir.join("edited.prtcad");
        let marked = dir.join("marked.prtcad");
        let saved = dir.join("saved.prtcad");
        let gone = dir.join("gone.prtcad");
        log(
            "a",
            &edited,
            serde_json::json!({"op": {}, "batch": Uuid::new_v4()}),
        );
        log(
            "b",
            &marked,
            serde_json::json!({"unsaved": [Uuid::new_v4()]}),
        );
        log("c", &saved, serde_json::json!({"unsaved": []}));
        log("d", &gone, serde_json::json!({"op": {}}));
        std::fs::remove_file(&gone).unwrap();

        let offered = with_logs(
            vec![Recoverable {
                copy: dir.join("copy.prtcad"),
                name: "copy".into(),
                file: None,
                saved_ms: u64::MAX,
                source: Source::Copy { untitled: None },
            }],
            doc_server::daemon::unsaved_logs_in(&dir),
            Vec::new(),
        );
        let names: Vec<(&str, bool)> = offered
            .iter()
            .map(|r| (r.name.as_str(), r.source == Source::DocumentLog))
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert_eq!(names[0], ("copy", false));
        assert!(names.contains(&("edited", true)) && names.contains(&("marked", true)));
        assert!(
            offered
                .iter()
                .all(|r| r.source != Source::DocumentLog || r.file == Some(r.copy.clone()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An untitled tab's copy and the log its daemon kept are one piece of
    /// work: offered once, as the log, under the copy's name, with the copy
    /// going along. A log with no copy is offered as an untitled session,
    /// and another tab's copy stays a copy.
    #[test]
    fn an_untitled_tabs_log_stands_for_its_copy() {
        let dir = std::env::temp_dir().join(format!("printcad-recovery-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let pid = u32::MAX - 1;
        let write = |tab: Uuid, name: &str| {
            let copy = dir.join(format!("{tab}.prtcad"));
            std::fs::write(&copy, b"copy").unwrap();
            let note = Note {
                name: name.to_string(),
                file: None,
                pid,
                saved_ms: 1,
            };
            std::fs::write(note_of(&copy), serde_json::to_vec(&note).unwrap()).unwrap();
            copy
        };
        let (logged, alone) = (Uuid::new_v4(), Uuid::new_v4());
        let logged_copy = write(logged, "Bracket");
        write(alone, "Alone");
        let log = |socket: PathBuf| doc_server::daemon::UntitledLog {
            socket,
            modified_ms: 2,
        };
        let crashed = doc_server::socket_path_for_untitled_of(pid, logged);
        let no_copy = doc_server::socket_path_for_untitled_of(pid, Uuid::new_v4());

        let offered = with_logs(
            left_behind_in(&dir),
            Vec::new(),
            vec![log(crashed.clone()), log(no_copy.clone())],
        );
        let entry = |key: &Path| offered.iter().find(|r| r.copy == key).cloned();
        assert_eq!(offered.len(), 3, "{offered:?}");
        let session = entry(&crashed).expect("the log is offered");
        assert_eq!(session.name, "Bracket");
        assert_eq!(
            session.source,
            Source::UntitledLog {
                copy_too: Some(logged_copy.clone())
            }
        );
        assert!(
            entry(&logged_copy).is_none(),
            "the copy is not offered twice"
        );
        assert_eq!(entry(&no_copy).expect("a log alone").name, "Untitled");
        assert!(matches!(
            entry(&dir.join(format!("{alone}.prtcad"))).map(|r| r.source),
            Some(Source::Copy { untitled: Some(_) })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
