//! JSON kept in the application's folders. An unreadable file is not a default.

use std::path::{Path, PathBuf};

use serde::{Serialize, de::DeserializeOwned};

#[derive(Debug, thiserror::Error)]
#[error("{} does not read: {reason}", path.display())]
pub struct Unreadable {
    path: PathBuf,
    reason: String,
}

/// Missing files start at the default. Every other read failure must prevent
/// the caller from saving its fallback over the file.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> Result<T, Unreadable> {
    let read = || -> Result<T, Box<dyn std::error::Error>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
            Err(err) => return Err(err.into()),
        };
        Ok(serde_json::from_slice(&bytes)?)
    };
    read().map_err(|err| Unreadable {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })
}

/// Keep a complete JSON value atomically, refusing to overwrite an unreadable file.
pub fn keep<T: Serialize + DeserializeOwned + Default>(
    path: &Path,
    value: &T,
) -> Result<(), super::SettingsError> {
    load::<T>(path)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    local_ipc::write_atomically(path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_files_are_not_overwritten() {
        let root = std::env::temp_dir().join(format!("settings-json-{}", std::process::id()));
        let path = root.join("settings.json");
        assert_eq!(load::<Vec<String>>(&path).unwrap(), Vec::<String>::new());
        keep(&path, &vec!["kept".to_owned()]).unwrap();
        for bad in ["{ broken", "{}", "null"] {
            std::fs::write(&path, bad).unwrap();
            assert!(load::<Vec<String>>(&path).is_err());
            assert!(keep(&path, &Vec::<String>::new()).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
