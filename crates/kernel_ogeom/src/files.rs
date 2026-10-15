//! How an imported file is read, by the adapter and by a bench importing
//! one: from the file system, unless the app hands it another reader. A
//! browser page has no file system and keeps the files its user picked in
//! memory.

use std::path::Path;
use std::sync::OnceLock;

/// A reader of a whole file.
pub type Reader = fn(&Path) -> std::io::Result<Vec<u8>>;

static READER: OnceLock<Reader> = OnceLock::new();

/// Read imported files through `reader` from now on. The first call wins;
/// a later one is ignored.
pub fn set_reader(reader: Reader) {
    let _ = READER.set(reader);
}

/// The bytes of `path`, through the reader set, else the file system.
pub fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    match READER.get() {
        Some(reader) => reader(path),
        None => std::fs::read(path),
    }
}
