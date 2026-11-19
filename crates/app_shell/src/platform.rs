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

/// What the worker answering on `slot` has sent: `Some(Ok)` its answer,
/// `Some(Err)` when it ended without one, `None` while it is still at
/// work or nothing is asked. Both answers clear `slot`, so a worker that
/// died leaves nothing waiting to keep frames coming.
pub(crate) fn take_answer<T>(
    slot: &mut Option<std::sync::mpsc::Receiver<T>>,
) -> Option<Result<T, std::sync::mpsc::RecvError>> {
    use std::sync::mpsc::{RecvError, TryRecvError};
    let answer = match slot.as_ref()?.try_recv() {
        Ok(answer) => Ok(answer),
        Err(TryRecvError::Empty) => return None,
        Err(TryRecvError::Disconnected) => Err(RecvError),
    };
    *slot = None;
    Some(answer)
}

/// The bytes of the file at `path`.
pub(crate) fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(path)
}

/// Write `bytes` as the file at `path`: into a file beside it, moved over
/// it once whole, so a failed or cut-off write leaves the file as it was.
pub(crate) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    local_ipc::write_atomically(path, bytes)
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

/// Where this run's private folders go: the user's own runtime folder,
/// else the temporary folder.
pub(crate) fn private_base() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute() && dir.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// A folder in `base` for this run's `what` files, made now under a name
/// no other run or user takes, and open to this user alone.
pub(crate) fn make_private_folder(base: &Path, what: &str) -> std::io::Result<PathBuf> {
    loop {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let folder = base.join(format!(
            "printcad-{what}-{}-{}",
            std::process::id(),
            &id[..12]
        ));
        #[cfg(unix)]
        let builder = {
            let mut builder = std::fs::DirBuilder::new();
            std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
            builder
        };
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        match builder.create(&folder) {
            Ok(()) => return Ok(folder),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
}

/// A new, empty file in `folder`: `stem.ext`, else `stem-2.ext` and on
/// (no dot when `ext` is empty), each made only where nothing is, so two
/// files of one name never share a path and nothing put there beforehand
/// is written through.
pub(crate) fn reserve_file(folder: &Path, stem: &str, ext: &str) -> std::io::Result<PathBuf> {
    let mut n = 1;
    loop {
        let stem = match n {
            1 => stem.to_string(),
            n => format!("{stem}-{n}"),
        };
        let path = match ext {
            "" => folder.join(stem),
            ext => folder.join(format!("{stem}.{ext}")),
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(err) => return Err(err),
        }
    }
}

/// This run's scratch folder, once made.
static SCRATCH: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Write `bytes` as a new file named after `name` in this run's private
/// scratch folder, for the kernel to read by path. The folder is made on
/// first use, and again should it go.
pub(crate) fn scratch_file(name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    scratch_file_in(&SCRATCH, &private_base(), name, bytes)
}

/// Remove this run's scratch folder with whatever is left in it, on the
/// way out: its files are the kernel's copies, read by then or no longer
/// wanted.
pub(crate) fn remove_scratch_folder() {
    remove_kept_folder(&SCRATCH);
}

fn remove_kept_folder(kept: &std::sync::Mutex<Option<PathBuf>>) {
    let folder = kept.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(folder) = folder {
        let _ = std::fs::remove_dir_all(folder);
    }
}

/// [`scratch_file`] in the folder `kept` holds, made in `base` when there
/// is none.
fn scratch_file_in(
    kept: &std::sync::Mutex<Option<PathBuf>>,
    base: &Path,
    name: &str,
    bytes: &[u8],
) -> std::io::Result<PathBuf> {
    let folder = {
        let mut kept = kept.lock().unwrap_or_else(|e| e.into_inner());
        match kept.clone().filter(|f| f.is_dir()) {
            Some(folder) => folder,
            None => {
                let folder = make_private_folder(base, "scratch")?;
                *kept = Some(folder.clone());
                folder
            }
        }
    };
    let name = Path::new(name);
    let stem = name
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "file".to_string());
    let ext = name.extension().unwrap_or_default().to_string_lossy();
    let path = reserve_file(&folder, &stem, &ext)?;
    write(&path, bytes)?;
    Ok(path)
}

#[cfg(test)]
mod private_tests {
    use super::*;

    /// Each folder is new and open to its user alone; two files of one
    /// name get a path each, and a link put where a file would go is
    /// passed over rather than written through.
    #[test]
    fn private_files_are_private_and_never_shared() {
        let base = std::env::temp_dir().join(format!("private-base-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&base).unwrap();
        let folder = make_private_folder(&base, "slicer").unwrap();
        let other = make_private_folder(&base, "slicer").unwrap();
        assert_ne!(folder, other, "a folder per call");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&folder), 0o700);
            let kept = std::sync::Mutex::new(None);
            let scratch = scratch_file_in(&kept, &base, "part.step", b"solid").unwrap();
            assert_eq!(mode(scratch.parent().unwrap()), 0o700);
            assert_eq!(std::fs::read(&scratch).unwrap(), b"solid");
            let again = scratch_file_in(&kept, &base, "part.step", b"x").unwrap();
            assert_ne!(again, scratch);
            remove_kept_folder(&kept);
            assert!(!scratch.parent().unwrap().exists(), "gone on the way out");
        }
        let first = reserve_file(&folder, "bracket", "3mf").unwrap();
        let second = reserve_file(&folder, "bracket", "3mf").unwrap();
        assert_ne!(first, second);
        assert!(first.is_file() && second.is_file());
        #[cfg(unix)]
        {
            let target = base.join("target");
            std::fs::write(&target, b"keep").unwrap();
            std::os::unix::fs::symlink(&target, other.join("part.stl")).unwrap();
            let path = reserve_file(&other, "part", "stl").unwrap();
            assert_eq!(path, other.join("part-2.stl"));
            write(&path, b"mesh").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"keep");
        }
        std::fs::remove_dir_all(&base).unwrap();
    }
}

#[cfg(test)]
mod answer_tests {
    use super::take_answer;

    /// A worker that ends without answering frees its slot, as one that
    /// answers does; one still at work keeps it.
    #[test]
    fn a_worker_that_dies_without_answering_frees_its_slot() {
        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        let mut slot = Some(rx);
        assert!(take_answer(&mut slot).is_none());
        assert!(slot.is_some(), "still at work");
        drop(tx);
        assert!(matches!(take_answer(&mut slot), Some(Err(_))));
        assert!(slot.is_none(), "nothing left waiting");

        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        let mut slot = Some(rx);
        tx.send(7).unwrap();
        assert_eq!(take_answer(&mut slot).unwrap().ok(), Some(7));
        assert!(slot.is_none());
        assert!(take_answer(&mut slot).is_none());
    }
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
