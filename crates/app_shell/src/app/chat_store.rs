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
}

/// Every document's chats, by file.
#[derive(Debug, Default)]
struct Store {
    documents: BTreeMap<String, Vec<SavedChat>>,
}

impl SavedChat {
    fn to_json(&self) -> Value {
        json!({"agent": self.agent, "session": self.session, "title": self.title})
    }

    fn from_json(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key)?.as_str().map(str::to_string);
        Some(Self {
            agent: text("agent")?,
            session: text("session")?,
            title: text("title").unwrap_or_default(),
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

fn load(path: &Path) -> Store {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|value| Store::from_json(&value))
        .unwrap_or_default()
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
    load(store)
        .documents
        .get(&key(file))
        .cloned()
        .unwrap_or_default()
}

fn keep_in(store: &Path, file: &Path, chats: Vec<SavedChat>) {
    let mut all = load(store);
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
}
