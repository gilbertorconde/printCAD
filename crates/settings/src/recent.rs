//! Recently opened documents and the directory file dialogs start in.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many documents the list keeps.
pub const RECENT_CAP: usize = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    /// Unix milliseconds of the last open or save.
    pub last_opened_ms: u64,
    /// File size when it was last touched; zero when unknown.
    #[serde(default)]
    pub size_bytes: u64,
}

impl RecentEntry {
    /// The document name: the file name without its document extension.
    pub fn name(&self) -> String {
        let file = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lowered = file.to_ascii_lowercase();
        for suffix in [".prtcad.zst", ".prtcad.gz", ".prtcad", ".json"] {
            if let Some(stripped) = lowered.strip_suffix(suffix) {
                return file[..stripped.len()].to_string();
            }
        }
        file
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentStore {
    /// Where file dialogs start.
    #[serde(default)]
    pub last_dir: Option<PathBuf>,
    /// Most recent first.
    #[serde(default)]
    pub files: Vec<RecentEntry>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl RecentStore {
    /// Parse the store's JSON. A bare directory string, the shape older
    /// builds wrote, becomes `last_dir`.
    pub fn from_json(text: &str) -> Self {
        if let Ok(store) = serde_json::from_str::<RecentStore>(text) {
            return store;
        }
        match serde_json::from_str::<String>(text) {
            Ok(dir) => Self {
                last_dir: Some(PathBuf::from(dir)),
                files: Vec::new(),
            },
            Err(_) => Self::default(),
        }
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::from_json(&text))
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), super::SettingsError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(file, self)?;
        Ok(())
    }

    /// Remember the directory of `path` for the next dialog.
    pub fn remember_dir(&mut self, path: &Path) {
        if let Some(dir) = path.parent() {
            self.last_dir = Some(dir.to_path_buf());
        }
    }

    /// Move `path` to the front of the list with the current time.
    pub fn touch(&mut self, path: &Path) {
        let size_bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        self.touch_at(path, now_ms(), size_bytes);
    }

    pub fn touch_at(&mut self, path: &Path, at_ms: u64, size_bytes: u64) {
        self.remember_dir(path);
        self.files.retain(|e| e.path != path);
        self.files.insert(
            0,
            RecentEntry {
                path: path.to_path_buf(),
                last_opened_ms: at_ms,
                size_bytes,
            },
        );
        self.files.truncate(RECENT_CAP);
    }

    pub fn remove(&mut self, path: &Path) {
        self.files.retain(|e| e.path != path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_bare_directory_string_seeds_last_dir() {
        let store = RecentStore::from_json("\"/home/someone/models/\"");
        assert_eq!(store.last_dir, Some(PathBuf::from("/home/someone/models/")));
        assert!(store.files.is_empty());
    }

    #[test]
    fn touch_dedupes_orders_and_caps() {
        let mut store = RecentStore::default();
        for i in 0..20 {
            store.touch_at(Path::new(&format!("/m/part{i}.prtcad")), i, 100);
        }
        assert_eq!(store.files.len(), RECENT_CAP);
        assert_eq!(store.files[0].path, PathBuf::from("/m/part19.prtcad"));
        store.touch_at(Path::new("/m/part10.prtcad"), 99, 5);
        assert_eq!(store.files[0].path, PathBuf::from("/m/part10.prtcad"));
        assert_eq!(
            store
                .files
                .iter()
                .filter(|e| e.path.ends_with("part10.prtcad"))
                .count(),
            1
        );
        assert_eq!(store.last_dir, Some(PathBuf::from("/m")));
        store.remove(Path::new("/m/part10.prtcad"));
        assert!(
            !store
                .files
                .iter()
                .any(|e| e.path.ends_with("part10.prtcad"))
        );
    }

    #[test]
    fn round_trips_through_json() {
        let mut store = RecentStore::default();
        store.touch_at(Path::new("/m/bracket.prtcad"), 1, 2048);
        let text = serde_json::to_string(&store).unwrap();
        assert_eq!(RecentStore::from_json(&text), store);
        assert_eq!(store.files[0].name(), "bracket");
    }
}
