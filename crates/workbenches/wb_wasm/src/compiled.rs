//! Compiled components, so a package is compiled once: kept in memory for
//! the process, by the hash of the component's bytes, and on disk in the
//! folder [`set_compiled_cache`] names, for the next start.
//!
//! On disk, `<folder>/wasmtime-<engine>/<content>.cwasm`: `<engine>` is a
//! hash of what makes a compiled form loadable by this engine (wasmtime's
//! version, target and settings, `Engine::precompile_compatibility_hash`),
//! `<content>` the sha256 of the component's bytes. A file holds a header
//! (a magic line, the content hash again, the sha256 of what follows) and
//! wasmtime's serialized component. Writing goes through a temporary file
//! renamed into place, so a reader never sees half a file; folders of
//! other engines are removed, and past [`DISK_ENTRIES`] files or
//! [`DISK_BYTES`] the least recently used go.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};
use wasmtime::Engine;
use wasmtime::component::Component;

/// Compiled forms kept in memory; past this many the least recently used
/// goes (one that is running stays alive with its workbench).
const MEMORY_ENTRIES: usize = 16;
/// Compiled forms kept on disk, at most this many files ...
const DISK_ENTRIES: usize = 64;
/// ... holding at most this many bytes.
const DISK_BYTES: u64 = 512 * 1024 * 1024;
/// A temporary file this old is one a writer left when it stopped.
const ABANDONED: Duration = Duration::from_secs(600);
const MAGIC: &[u8; 16] = b"printcad-cwasm1\n";
const HEADER: usize = MAGIC.len() + 64;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Name the folder compiled components are kept in across runs: one the
/// app alone writes, such as its cache folder, never one a package archive
/// unpacks into. The first one named holds; without one they are kept
/// for the process only.
pub fn set_compiled_cache(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// Where a component came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Memory,
    Disk,
    Compiled,
}

type Key = [u8; 32];
type Slot = Arc<OnceLock<Result<Component, String>>>;

/// Each component's slot and the tick it was last asked for at.
#[derive(Default)]
struct Memory {
    tick: u64,
    slots: HashMap<Key, (u64, Slot)>,
}

pub(crate) struct Cache {
    /// The folder on disk, when it is not [`set_compiled_cache`]'s.
    dir: Option<PathBuf>,
    /// Whether the folder is [`set_compiled_cache`]'s.
    named: bool,
    max_entries: usize,
    max_bytes: u64,
    /// A slot is filled once, by the first asker, while the others wait.
    memory: Mutex<Memory>,
}

pub(crate) static CACHE: std::sync::LazyLock<Cache> = std::sync::LazyLock::new(|| Cache {
    named: true,
    ..Cache::new(None, DISK_ENTRIES, DISK_BYTES)
});

impl Cache {
    /// A cache keeping its files in `dir`, memory only without one.
    pub(crate) fn new(dir: Option<PathBuf>, max_entries: usize, max_bytes: u64) -> Self {
        Self {
            dir,
            named: false,
            max_entries,
            max_bytes,
            memory: Mutex::default(),
        }
    }

    fn dir(&self) -> Option<&Path> {
        if self.named {
            DIR.get().map(PathBuf::as_path)
        } else {
            self.dir.as_deref()
        }
    }

    /// The component in `bytes` for `engine`, compiled only when neither
    /// memory nor disk holds it.
    pub(crate) fn component(
        &self,
        engine: &Engine,
        bytes: &[u8],
    ) -> Result<(Component, Origin), String> {
        let key: Key = Sha256::digest(bytes).into();
        let slot = {
            let mut memory = self.memory.lock().unwrap_or_else(|e| e.into_inner());
            let Memory { tick, slots } = &mut *memory;
            *tick += 1;
            let now = *tick;
            let slot = slots
                .entry(key)
                .and_modify(|(used, _)| *used = now)
                .or_insert_with(|| (now, Slot::default()))
                .1
                .clone();
            while slots.len() > MEMORY_ENTRIES {
                let Some(oldest) = slots.iter().min_by_key(|(_, (used, _))| *used) else {
                    break;
                };
                let oldest = *oldest.0;
                slots.remove(&oldest);
            }
            slot
        };
        let mut origin = Origin::Memory;
        let component = slot.get_or_init(|| {
            let (component, from) = self.load(engine, bytes, &key);
            origin = from;
            component
        });
        component.clone().map(|c| (c, origin))
    }

    /// From disk when this engine wrote it there from the same bytes,
    /// compiled (and written) otherwise.
    fn load(
        &self,
        engine: &Engine,
        bytes: &[u8],
        key: &Key,
    ) -> (Result<Component, String>, Origin) {
        let file = self.dir().map(|dir| {
            dir.join(engine_folder(engine))
                .join(format!("{}.cwasm", hex(key)))
        });
        if let Some(file) = &file {
            if let Some(component) = read(engine, file, key) {
                return (Ok(component), Origin::Disk);
            }
            // Missing, damaged or another engine's: it is written anew.
            let _ = fs::remove_file(file);
        }
        let component = match Component::new(engine, bytes) {
            Ok(component) => component,
            Err(e) => return (Err(format!("{e:#}")), Origin::Compiled),
        };
        if let Some(file) = &file
            && let Ok(serialized) = component.serialize()
        {
            self.write(file, key, &serialized);
        }
        (Ok(component), Origin::Compiled)
    }

    fn write(&self, file: &Path, key: &Key, serialized: &[u8]) {
        let Some(folder) = file.parent() else {
            return;
        };
        if private_folder(folder).is_err() {
            return;
        }
        let mut contents = Vec::with_capacity(HEADER + serialized.len());
        contents.extend_from_slice(MAGIC);
        contents.extend_from_slice(key);
        contents.extend_from_slice(&Sha256::digest(serialized));
        contents.extend_from_slice(serialized);
        let nanos = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = folder.join(format!("{}.{}-{nanos}.tmp", hex(key), std::process::id()));
        if fs::write(&temporary, &contents).is_err() || fs::rename(&temporary, file).is_err() {
            let _ = fs::remove_file(&temporary);
            return;
        }
        self.tidy(folder, file);
    }

    /// Remove other engines' folders, abandoned temporary files, and the
    /// least recently used files past the bounds, never `kept`.
    fn tidy(&self, folder: &Path, kept: &Path) {
        if let Some(root) = folder.parent()
            && let Ok(entries) = fs::read_dir(root)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if path != folder && name.to_string_lossy().starts_with("wasmtime-") {
                    let _ = fs::remove_dir_all(&path);
                }
            }
        }
        let Ok(entries) = fs::read_dir(folder) else {
            return;
        };
        let now = SystemTime::now();
        let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            match path.extension().and_then(|e| e.to_str()) {
                Some("cwasm") if path != kept => files.push((modified, meta.len(), path)),
                Some("tmp") if now.duration_since(modified).unwrap_or_default() > ABANDONED => {
                    let _ = fs::remove_file(&path);
                }
                _ => {}
            }
        }
        let kept_bytes = fs::metadata(kept).map(|m| m.len()).unwrap_or(0);
        files.sort();
        let mut count = files.len() + 1;
        let mut total = kept_bytes + files.iter().map(|f| f.1).sum::<u64>();
        for (_, size, path) in files {
            if count <= self.max_entries && total <= self.max_bytes {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                count -= 1;
                total -= size;
            }
        }
    }
}

/// The compiled component in `file`, when the file is whole and was
/// written from the bytes `key` names; it is marked used.
fn read(engine: &Engine, file: &Path, key: &Key) -> Option<Component> {
    let mut contents = Vec::new();
    fs::File::open(file).ok()?.read_to_end(&mut contents).ok()?;
    let (header, serialized) = contents.split_at_checked(HEADER)?;
    let (magic, hashes) = header.split_at(MAGIC.len());
    let (named, digest) = hashes.split_at(32);
    if magic != MAGIC || named != key || digest != Sha256::digest(serialized).as_slice() {
        return None;
    }
    // SAFETY: deserializing runs `serialized` as native code, so it must be
    // what `Component::serialize` produced. The file is in the folder the
    // app named for its own compiled components, private to the user,
    // under the folder of this engine's compatibility hash; it was written
    // whole (a rename puts it in place), and its header carries the hash of
    // the component it was compiled from, which is the one being loaded,
    // and a digest of the serialized form that matches it, so a truncated,
    // damaged or misplaced file never gets here. Wasmtime checks again that
    // its version and settings are this engine's. A process able to write
    // in that folder runs as the user and could change the app itself.
    let component = unsafe { Component::deserialize(engine, serialized) }.ok()?;
    if let Ok(handle) = fs::OpenOptions::new().write(true).open(file) {
        let _ = handle.set_modified(SystemTime::now());
    }
    Some(component)
}

/// The folder this engine's compiled forms go in.
fn engine_folder(engine: &Engine) -> String {
    use std::hash::Hash;
    struct Sha(Sha256);
    impl std::hash::Hasher for Sha {
        fn finish(&self) -> u64 {
            0
        }
        fn write(&mut self, bytes: &[u8]) {
            self.0.update(bytes);
        }
    }
    let mut sha = Sha(Sha256::new());
    engine.precompile_compatibility_hash().hash(&mut sha);
    let digest: Key = sha.0.finalize().into();
    format!("wasmtime-{}", &hex(&digest)[..16])
}

/// `folder` and its parents, made readable by the user alone where the
/// system has such permissions.
fn private_folder(folder: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(folder)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> &'static Engine {
        &crate::engine::ENGINE
    }

    /// A small component, different for each `functions`.
    fn component(functions: usize) -> Vec<u8> {
        let body = "(func)".repeat(functions);
        format!("(component (core module {body}))").into_bytes()
    }

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("printcad-compiled-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn origin(cache: &Cache, bytes: &[u8]) -> Origin {
        cache.component(engine(), bytes).unwrap().1
    }

    fn cwasm(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(dir.join(engine_folder(engine())))
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.retain(|p| p.extension().is_some_and(|e| e == "cwasm"));
        files.sort();
        files
    }

    #[test]
    fn a_component_is_compiled_once_then_read_back_from_memory_and_disk() {
        let dir = folder("once");
        let bytes = component(1);
        let cache = Cache::new(Some(dir.clone()), 8, u64::MAX);
        assert_eq!(origin(&cache, &bytes), Origin::Compiled);
        assert_eq!(origin(&cache, &bytes), Origin::Memory);
        let next_start = Cache::new(Some(dir.clone()), 8, u64::MAX);
        assert_eq!(origin(&next_start, &bytes), Origin::Disk);

        // Changed bytes are another component, compiled.
        assert_eq!(origin(&next_start, &component(2)), Origin::Compiled);
        assert_eq!(cwasm(&dir).len(), 2);

        // Without a folder the process keeps it all the same.
        let no_disk = Cache::new(None, 8, u64::MAX);
        assert_eq!(origin(&no_disk, &bytes), Origin::Compiled);
        assert_eq!(origin(&no_disk, &bytes), Origin::Memory);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_or_misplaced_file_is_compiled_again_not_loaded() {
        let dir = folder("damaged");
        let bytes = component(3);
        assert_eq!(
            origin(&Cache::new(Some(dir.clone()), 8, u64::MAX), &bytes),
            Origin::Compiled
        );
        let file = cwasm(&dir).pop().unwrap();
        let good = fs::read(&file).unwrap();
        let mut damaged = good.clone();
        *damaged.last_mut().unwrap() ^= 0x40;
        let other = component(4);
        let other_name = file.with_file_name(format!("{}.cwasm", hex(&Sha256::digest(&other))));
        for (path, contents) in [
            (&file, damaged),
            (&file, good[..good.len() / 2].to_vec()),
            // A whole file under another component's name.
            (&other_name, good.clone()),
        ] {
            fs::write(path, contents).unwrap();
            let fresh = Cache::new(Some(dir.clone()), 8, u64::MAX);
            let wanted = if path == &file { &bytes } else { &other };
            assert_eq!(origin(&fresh, wanted), Origin::Compiled);
            // What took its place is whole.
            let fresh = Cache::new(Some(dir.clone()), 8, u64::MAX);
            assert_eq!(origin(&fresh, wanted), Origin::Disk);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_least_recently_used_and_other_engines_files_go() {
        let dir = folder("bounded");
        let stale = dir.join("wasmtime-0000000000000000");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("x.cwasm"), b"old").unwrap();
        let (a, b, c) = (component(5), component(6), component(7));
        let cache = || Cache::new(Some(dir.clone()), 2, u64::MAX);
        origin(&cache(), &a);
        origin(&cache(), &b);
        // Reading `a` back marks it used, so `b` is the oldest.
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(origin(&cache(), &a), Origin::Disk);
        origin(&cache(), &c);
        let left = cwasm(&dir);
        assert_eq!(left.len(), 2);
        assert!(
            !left
                .iter()
                .any(|p| p.ends_with(format!("{}.cwasm", hex(&Sha256::digest(&b)))))
        );
        assert!(!stale.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn callers_asking_at_once_share_one_compile() {
        let cache = Cache::new(None, 8, u64::MAX);
        let bytes = component(8);
        let origins: Vec<Origin> = std::thread::scope(|s| {
            let asks: Vec<_> = (0..4).map(|_| s.spawn(|| origin(&cache, &bytes))).collect();
            asks.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(
            origins.iter().filter(|o| **o == Origin::Compiled).count(),
            1
        );
    }
}
