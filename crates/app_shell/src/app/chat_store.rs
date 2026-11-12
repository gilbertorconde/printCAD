//! The chats kept with each document's file: which agent, which of its
//! sessions, and the chat's title, so opening the file again brings its
//! open chats back where they left off, and a closed one can be reopened
//! from the assistant's history.
//!
//! They live in the application's own folder (`chats.json`), not in the
//! document: a session means something only to the agent on this machine,
//! and a file handed to someone else carries no one's conversations.
//! The file holds two maps by file, `documents` (the chats open when the
//! file was last seen) and `closed` (those closed since, for the history),
//! so a closed chat never comes back open.

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
    /// a chat kept without one resumes in the file's folder.
    pub folder: Option<PathBuf>,
    /// When the chat was last used, in milliseconds since the epoch;
    /// unknown for a chat kept without it.
    pub used: Option<u64>,
}

/// Every document's chats, by file: open and closed apart.
#[derive(Debug, Default, Clone, PartialEq)]
struct Store {
    documents: BTreeMap<String, Vec<SavedChat>>,
    closed: BTreeMap<String, Vec<SavedChat>>,
}

/// A closed chat in the history: the file it was kept with, by the
/// store's key.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClosedChat {
    pub file: String,
    pub chat: SavedChat,
}

/// Now, in milliseconds since the epoch.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
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
        if let Some(used) = self.used {
            out["used"] = json!(used);
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
            used: value.get("used").and_then(Value::as_u64),
        })
    }
}

fn map_from_json(value: Option<&Value>) -> BTreeMap<String, Vec<SavedChat>> {
    value
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
        .unwrap_or_default()
}

fn map_to_json(map: &BTreeMap<String, Vec<SavedChat>>) -> Value {
    Value::Object(
        map.iter()
            .filter(|(_, chats)| !chats.is_empty())
            .map(|(file, chats)| {
                (
                    file.clone(),
                    Value::Array(chats.iter().map(SavedChat::to_json).collect()),
                )
            })
            .collect(),
    )
}

impl Store {
    fn from_json(value: &Value) -> Self {
        Self {
            documents: map_from_json(value.get("documents")),
            closed: map_from_json(value.get("closed")),
        }
    }

    fn to_json(&self) -> Value {
        let mut out = json!({"documents": map_to_json(&self.documents)});
        if self.closed.values().any(|chats| !chats.is_empty()) {
            out["closed"] = map_to_json(&self.closed);
        }
        out
    }

    /// Take the chat in `session` out of `file`'s open and closed lists.
    fn drop_chat(&mut self, file: &str, session: &str) {
        for map in [&mut self.documents, &mut self.closed] {
            if let Some(chats) = map.get_mut(file) {
                chats.retain(|c| c.session != session);
                if chats.is_empty() {
                    map.remove(file);
                }
            }
        }
    }
}

/// What the store is keyed by: the file as an absolute path.
pub(crate) fn key(file: &Path) -> String {
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
    settings::json_file::load::<Value>(path)
        .map(|value| Store::from_json(&value))
        .map_err(|err| err.to_string())
}

/// The open chats kept with `file`.
pub(crate) fn chats_of(file: &Path) -> Vec<SavedChat> {
    store_path()
        .map(|path| chats_in(&path, file))
        .unwrap_or_default()
}

/// Keep `chats` as `file`'s open chats, in place of what it had.
pub(crate) fn keep(file: &Path, chats: Vec<SavedChat>) {
    if let Some(path) = store_path() {
        keep_in(&path, file, chats);
    }
}

/// Keep `chat` with `file` as closed: out of its open chats, first in its
/// history.
pub(crate) fn close(file: &Path, chat: SavedChat) {
    if let Some(path) = store_path() {
        close_in(&path, &key(file), chat);
    }
}

/// Take the chat in `session` out of the store for good.
pub(crate) fn forget(file: &str, session: &str) {
    if let Some(path) = store_path() {
        forget_in(&path, file, session);
    }
}

/// Every file's closed chats, newest first.
pub(crate) fn history() -> Vec<ClosedChat> {
    store_path()
        .map(|path| history_in(&path))
        .unwrap_or_default()
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

pub(crate) fn history_in(store: &Path) -> Vec<ClosedChat> {
    let all = match load(store) {
        Ok(all) => all,
        Err(err) => {
            crate::log_panel::warn(format!("The past chats are not listed: {err}"));
            return Vec::new();
        }
    };
    let mut out: Vec<ClosedChat> = all
        .closed
        .into_iter()
        .flat_map(|(file, chats)| {
            chats.into_iter().map(move |chat| ClosedChat {
                file: file.clone(),
                chat,
            })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.chat.used));
    out
}

/// Change the store with `edit`, which says whether it changed anything.
/// A store that does not read is left as it is, rather than written over
/// with this document's chats alone.
fn change(store: &Path, edit: impl FnOnce(&mut Store) -> bool) {
    let mut all = match load(store) {
        Ok(all) => all,
        Err(err) => {
            crate::log_panel::warn(format!("The document's chats are not kept: {err}"));
            return;
        }
    };
    if !edit(&mut all) {
        return;
    }
    if let Err(err) = settings::json_file::keep(store, &all.to_json()) {
        crate::log_panel::warn(format!("Could not keep the document's chats: {err}"));
    }
}

fn keep_in(store: &Path, file: &Path, chats: Vec<SavedChat>) {
    let key = key(file);
    change(store, |all| {
        if all
            .documents
            .get(&key)
            .map_or(chats.is_empty(), |c| *c == chats)
        {
            return false;
        }
        if chats.is_empty() {
            all.documents.remove(&key);
        } else {
            all.documents.insert(key, chats);
        }
        true
    });
}

pub(crate) fn close_in(store: &Path, file: &str, chat: SavedChat) {
    change(store, |all| {
        all.drop_chat(file, &chat.session);
        all.closed
            .entry(file.to_string())
            .or_default()
            .insert(0, chat);
        true
    });
}

fn forget_in(store: &Path, file: &str, session: &str) {
    change(store, |all| {
        let before = all.clone();
        all.drop_chat(file, session);
        *all != before
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(session: &str, folder: Option<PathBuf>) -> SavedChat {
        SavedChat {
            agent: "Claude".into(),
            session: session.into(),
            title: "Chat 1".into(),
            named: session == "s2",
            folder,
            used: None,
        }
    }

    #[test]
    fn each_file_keeps_its_own_chats() {
        let dir = std::env::temp_dir().join(format!("printcad-chats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("chats.json");
        let (a, b) = (dir.join("a.prtcad"), dir.join("b.prtcad"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let chat = |session: &str| chat(session, Some(dir.clone()));
        keep_in(&store, &a, vec![chat("s1"), chat("s2")]);
        keep_in(&store, &b, vec![chat("s3")]);
        assert_eq!(chats_in(&store, &a), [chat("s1"), chat("s2")]);
        // The same file by another path is the same document.
        assert_eq!(
            chats_in(&store, &dir.join(".").join("b.prtcad")),
            [chat("s3")]
        );

        // A closed chat leaves the open ones for the history, newest
        // first across files, and never comes back open.
        let used = |mut c: SavedChat, at: u64| {
            c.used = Some(at);
            c
        };
        close_in(&store, &key(&a), used(chat("s1"), 10));
        close_in(&store, &key(&b), used(chat("s3"), 20));
        assert_eq!(chats_in(&store, &a), [chat("s2")]);
        assert!(chats_in(&store, &b).is_empty());
        let history: Vec<_> = history_in(&store)
            .into_iter()
            .map(|c| (c.file, c.chat.session))
            .collect();
        assert_eq!(history, [(key(&b), "s3".into()), (key(&a), "s1".into())]);
        // Keeping the open chats leaves the closed ones alone.
        keep_in(&store, &a, Vec::new());
        assert!(chats_in(&store, &a).is_empty());
        assert_eq!(history_in(&store).len(), 2);
        // Forgotten, a chat is gone from the store.
        forget_in(&store, &key(&a), "s1");
        assert_eq!(history_in(&store).len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A store kept before closed chats and use times were recorded reads
    /// as open chats, with no time and, without a folder, resuming in its
    /// file's folder.
    #[test]
    fn an_earlier_store_still_reads() {
        let store = Store::from_json(&json!({"documents": {"/x/a.prtcad": [
            {"agent": "Claude", "session": "s1", "title": "Chat 1"},
            {"agent": "Claude", "session": "s2", "title": "Chat 2", "folder": "/home/u", "named": true}
        ]}}));
        let chats = &store.documents["/x/a.prtcad"];
        assert_eq!(chats[0].folder, None);
        assert_eq!(chats[1].folder, Some(PathBuf::from("/home/u")));
        assert!(chats.iter().all(|c| c.used.is_none()) && store.closed.is_empty());
        assert_eq!(Store::from_json(&store.to_json()), store);
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
        keep_in(&store, &file, vec![chat("s1", None)]);
        close_in(&store, &key(&file), chat("s1", None));
        assert_eq!(std::fs::read_to_string(&store).unwrap(), "{ not json");
        assert!(chats_in(&store, &file).is_empty());
        // No file yet is no trouble: the first chats make it.
        std::fs::remove_file(&store).unwrap();
        assert!(load(&store).is_ok_and(|s| s.documents.is_empty()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
