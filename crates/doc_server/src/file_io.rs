//! File writes shared by the daemon and direct-file backend.

/// Write beside the target, synchronize the file, then rename over it.
pub(crate) fn write_atomically(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("prtcad.writing");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creation_replacement_and_failures() {
        let root = std::env::temp_dir().join(format!("printcad-save-{}", uuid::Uuid::new_v4()));
        let path = root.join("nested/document.prtcad");
        for bytes in [b"first".as_slice(), b"replacement", b""] {
            write_atomically(&path, bytes).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            assert!(!path.with_extension("prtcad.writing").exists());
        }
        // A blocked temporary path must leave the existing document intact.
        std::fs::create_dir(path.with_extension("prtcad.writing")).unwrap();
        assert!(write_atomically(&path, b"not saved").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        // A parent that is a file refuses creation; a directory refuses replacement.
        for target in [path.join("child.prtcad"), root.join("nested")] {
            assert!(write_atomically(&target, b"not saved").is_err());
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        std::fs::remove_dir_all(root).unwrap();
    }
}
