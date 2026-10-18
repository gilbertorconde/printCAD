//! Starting work beside the window, and reading and writing the user's
//! files: a named thread, the file system.

use std::path::{Path, PathBuf};

/// Work started beside the window, which an exit can wait for.
pub(crate) struct Job {
    handle: std::thread::JoinHandle<()>,
}

impl Job {
    /// Whether the work is done.
    pub(crate) fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    /// Wait for the work to end.
    pub(crate) fn join(self) {
        let _ = self.handle.join();
    }
}

/// Run `work` on a thread named `name`; its answer goes where the caller
/// arranged (a channel read on a later frame).
pub(crate) fn spawn(name: &str, work: impl FnOnce() + Send + 'static) -> std::io::Result<Job> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(work)
        .map(|handle| Job { handle })
}

/// The bytes of the file at `path`.
pub(crate) fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// Write `bytes` as the file at `path`.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

/// Take the file at `path` away; one that is not there is no error.
pub(crate) fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// The files directly in folder `dir`.
pub(crate) fn list(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Whether there is a file at `path`.
pub(crate) fn exists(path: &Path) -> bool {
    path.is_file()
}

/// Whether the file at `path` cannot be written in place: marked read-only,
/// or closed to this user for writing. A file that is not there is not.
pub(crate) fn read_only(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    if meta.permissions().readonly() {
        return true;
    }
    // Write bits for others alone (another user's file): opening it to
    // write, which changes nothing, asks the system whether this user
    // may.
    #[cfg(unix)]
    return matches!(
        std::fs::OpenOptions::new().write(true).open(path),
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied
    );
    #[cfg(not(unix))]
    false
}

/// Write `bytes` as a file named `name` under the temporary folder, for the
/// kernel to read by path.
pub(crate) fn scratch_file(folder: &str, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("printcad").join(folder);
    let path = dir.join(name);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod read_only_tests {
    use super::read_only;

    #[test]
    fn a_file_marked_read_only_is_and_a_missing_one_is_not() {
        let path = std::env::temp_dir().join(format!(
            "printcad_read_only_{}_{:?}.prtcad",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, b"x").expect("write");
        assert!(!read_only(&path));

        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions.clone()).expect("mark read-only");
        assert!(read_only(&path));

        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).expect("mark writable");
        assert!(!read_only(&path));
        let _ = std::fs::remove_file(&path);
        assert!(!read_only(&path));
    }
}
