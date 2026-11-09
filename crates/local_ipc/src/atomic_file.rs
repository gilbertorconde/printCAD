//! Replace a file only after its complete contents have reached disk.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Write beside the target, synchronize the file, then rename over it.
/// Failed writes leave the target untouched and remove their temporary file.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_with(path, |file| file.write_all(bytes))
}

fn write_with(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let (tmp, mut file) = loop {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let tmp = parent.join(format!(".printcad-{}-{id}.writing", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    };
    let result = write(&mut file).and_then(|()| file.sync_all());
    drop(file);
    let result = result.and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_and_failed_partial_writes() {
        let root = std::env::temp_dir().join(format!("atomic-file-{}", std::process::id()));
        let path = root.join("nested/document.prtcad");
        for bytes in [b"first".as_slice(), b"replacement", b""] {
            write_atomically(&path, bytes).unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        for target in [&path, &root.join("missing")] {
            assert!(
                write_with(target, |file| {
                    file.write_all(b"partial")?;
                    Err(io::Error::other("injected write failure"))
                })
                .is_err()
            );
        }
        assert_eq!(fs::read(&path).unwrap(), b"");
        assert!(!root.join("missing").exists());
        for target in [path.join("child"), root.join("nested")] {
            assert!(write_atomically(&target, b"not saved").is_err());
        }
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
