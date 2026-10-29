//! The chats kept with each document's file: which agent, which of its
//! sessions, and the chat's title, so opening the file again brings its
//! chats back where they left off.
//!
//! They live in the application's own folder (`chats.json`), not in the
//! document: a session means something only to the agent on this machine,
//! and a file handed to someone else carries no one's conversations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// A chat kept with a document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SavedChat {
    /// The agent's name in Preferences.
    pub agent: String,
    /// The agent's session id.
    pub session: String,
    pub title: String,
    /// The user named the chat: no title the agent gives replaces it.
    pub named: bool,
    /// The folder the agent ran the session in, which it is resumed in;
    /// a chat kept before this was recorded resumes in the file's folder.
    pub folder: Option<PathBuf>,
}

/// Every document's chats, by file.
#[derive(Debug, Default)]
struct Store {
    documents: BTreeMap<String, Vec<SavedChat>>,
}

impl SavedChat {
    fn to_json(&self) -> Value {
        let mut out = json!({"agent": self.agent, "session": self.session, "title": self.title});
        if let Some(folder) = &self.folder {
            out["folder"] = json!(folder.display().to_string());
        }
        if self.named {
            out["named"] = json!(true);
        }
        out
    }

    fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key)?.as_str().map(str::to_string);
        Some(Self {
            agent: text("agent")?,
            session: text("session")?,
            title: text("title").unwrap_or_default(),
            named: value.get("named").and_then(Value::as_bool).unwrap_or(false),
            folder: text("folder").map(PathBuf::from),
        })
    }
}

impl Store {
    fn from_json(value: &Value) -> Self {
        let documents = value
            .get("documents")
            .and_then(Value::as_object)
            .map(|docs| {
                docs.iter()
                    .map(|(file, chats)| {
                        let chats = chats
                            .as_array()
                            .map(|list| list.iter().filter_map(SavedChat::from_json).collect())
                            .unwrap_or_default();
                        (file.clone(), chats)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { documents }
    }

    fn to_json(&self) -> Value {
        let documents: serde_json::Map<String, Value> = self
            .documents
            .iter()
            .map(|(file, chats)| {
                (
                    file.clone(),
                    Value::Array(chats.iter().map(SavedChat::to_json).collect()),
                )
            })
            .collect();
        json!({"documents": documents})
    }
}

/// What the store is keyed by: the file as an absolute path.
fn key(file: &Path) -> String {
    std::fs::canonicalize(file)
        .unwrap_or_else(|_| file.to_path_buf())
        .display()
        .to_string()
}

fn store_path() -> Option<PathBuf> {
    settings::SettingsStore::chats_file_path().ok()
}

/// The store at `path`: empty when there is no file yet, an error when
/// there is one that does not read, which must not be written over.
fn load(path: &Path) -> Result<Store, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Err(err) => return Err(format!("{} does not read: {err}", path.display())),
    };
    serde_json::from_slice::<Value>(&bytes)
        .map(|value| Store::from_json(&value))
        .map_err(|err| format!("{} does not read: {err}", path.display()))
}

/// The chats kept with `file`.
pub(crate) fn chats_of(file: &Path) -> Vec<SavedChat> {
    store_path()
        .map(|path| chats_in(&path, file))
        .unwrap_or_default()
}

/// Keep `chats` with `file`, in place of what it had.
pub(crate) fn keep(file: &Path, chats: Vec<SavedChat>) {
    if let Some(path) = store_path() {
        keep_in(&path, file, chats);
    }
}

fn chats_in(store: &Path, file: &Path) -> Vec<SavedChat> {
    match load(store) {
        Ok(all) => all.documents.get(&key(file)).cloned().unwrap_or_default(),
        Err(err) => {
            crate::log_panel::warn(format!("The document's chats are not restored: {err}"));
            Vec::new()
        }
    }
}

/// Keep `chats` with `file` in `store`. A store that does not read is left
/// as it is, rather than written over with this document's chats alone.
fn keep_in(store: &Path, file: &Path, chats: Vec<SavedChat>) {
    let mut all = match load(store) {
        Ok(all) => all,
        Err(err) => {
            crate::log_panel::warn(format!("The document's chats are not kept: {err}"));
            return;
        }
    };
    let key = key(file);
    if all.documents.get(&key) == Some(&chats)
        || (chats.is_empty() && !all.documents.contains_key(&key))
    {
        return;
    }
    if chats.is_empty() {
        all.documents.remove(&key);
    } else {
        all.documents.insert(key, chats);
    }
    match serde_json::to_vec_pretty(&all.to_json()) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(store, bytes) {
                crate::log_panel::warn(format!("Could not keep the document's chats: {err}"));
            }
        }
        Err(err) => crate::log_panel::warn(format!("Could not keep the document's chats: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_file_keeps_its_own_chats() {
        let dir = std::env::temp_dir().join(format!("printcad-chats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("chats.json");
        let (a, b) = (dir.join("a.prtcad"), dir.join("b.prtcad"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let chat = |session: &str| SavedChat {
            agent: "Claude".into(),
            session: session.into(),
            title: "Chat 1".into(),
            named: session == "s2",
            folder: Some(dir.clone()),
        };
        keep_in(&store, &a, vec![chat("s1"), chat("s2")]);
        keep_in(&store, &b, vec![chat("s3")]);
        assert_eq!(chats_in(&store, &a), [chat("s1"), chat("s2")]);
        // The same file by another path is the same document.
        assert_eq!(
            chats_in(&store, &dir.join(".").join("b.prtcad")),
            [chat("s3")]
        );
        keep_in(&store, &a, Vec::new());
        assert!(chats_in(&store, &a).is_empty());
        assert_eq!(chats_in(&store, &b).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A chat kept before its folder was recorded reads with none, and the
    /// chat resumes in its file's folder.
    #[test]
    fn a_chat_kept_without_its_folder_still_reads() {
        let store = Store::from_json(&json!({"documents": {"/x/a.prtcad": [
            {"agent": "Claude", "session": "s1", "title": "Chat 1"},
            {"agent": "Claude", "session": "s2", "title": "Chat 2", "folder": "/home/u"}
        ]}}));
        let chats = &store.documents["/x/a.prtcad"];
        assert_eq!(chats[0].folder, None);
        assert_eq!(chats[1].folder, Some(PathBuf::from("/home/u")));
        let again = Store::from_json(&store.to_json());
        assert_eq!(again.documents["/x/a.prtcad"], *chats);
    }

    /// A store that does not read is never written over: the chats of
    /// every other document stay for whoever mends it.
    #[test]
    fn a_store_that_does_not_read_is_left_as_it_is() {
        let dir = std::env::temp_dir().join(format!("printcad-chats-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("chats.json");
        let file = dir.join("a.prtcad");
        std::fs::write(&file, "").unwrap();
        std::fs::write(&store, "{ not json").unwrap();
        let chat = SavedChat {
            agent: "Claude".into(),
            session: "s1".into(),
            title: "Chat 1".into(),
            named: false,
            folder: None,
        };
        keep_in(&store, &file, vec![chat]);
        assert_eq!(std::fs::read_to_string(&store).unwrap(), "{ not json");
        assert!(chats_in(&store, &file).is_empty());
        // No file yet is no trouble: the first chats make it.
        std::fs::remove_file(&store).unwrap();
        assert!(load(&store).is_ok_and(|s| s.documents.is_empty()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
