//! Small writes into a feature's payload.
//!
//! A [`DataPatch`] is a list of [`PatchWrite`]s applied in order to a
//! feature's JSON data: a value set or taken away at a path, or an item
//! put into or taken out of a list whose items carry ids. A path names a
//! list's item by its id ([`PathStep::Item`]), never by its place, so a
//! write still finds its item when an earlier one went.
//!
//! A patch applies whole or not at all ([`DataPatch::apply`]): a write
//! whose path is missing (a peer took the item away first) puts back the
//! writes before it and the data stands as it was. Applying answers the
//! patch that undoes it, worked out write by write from the data each
//! write found.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One step of a path into a payload: an object's key, or the item of a
/// list whose id is `id` (its own `id` field, or for an item written as
/// `{variant: {...}}` its variant's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathStep {
    Key(String),
    Item { id: String },
}

impl PathStep {
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key(key.into())
    }

    pub fn item(id: impl ToString) -> Self {
        Self::Item { id: id.to_string() }
    }
}

/// One write of a [`DataPatch`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchWrite {
    /// `value` at `path`: an object's key set (made when missing), or a
    /// list's item replaced by one with the same id.
    Set { path: Vec<PathStep>, value: Value },
    /// The key or item at `path` taken away; it must be there.
    Remove { path: Vec<PathStep> },
    /// `value` put into the list at `list` right after the item `after`
    /// names, or first for `None`. Its id is one the list does not hold:
    /// made when the edit was, and the list is not looked through for it,
    /// which would cost a walk of the whole list for each item added.
    Insert {
        list: Vec<PathStep>,
        after: Option<String>,
        value: Value,
    },
}

impl PatchWrite {
    /// The key or item the write changes, for telling which writes touch
    /// one another; `None` for an insert whose item has no id.
    fn target(&self) -> Option<std::borrow::Cow<'_, [PathStep]>> {
        match self {
            Self::Set { path, .. } | Self::Remove { path } => Some(path.as_slice().into()),
            Self::Insert { list, value, .. } => {
                let mut path = list.clone();
                path.push(PathStep::item(item_id(value)?));
                Some(path.into())
            }
        }
    }
}

/// Why a patch did not apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchError {
    /// The write that did not apply, counted from 0.
    pub write: usize,
    pub reason: &'static str,
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "write {} of the patch: {}", self.write, self.reason)
    }
}

impl std::error::Error for PatchError {}

/// Writes into a feature's payload, applied in order. See the module
/// docs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DataPatch {
    writes: VecDeque<PatchWrite>,
}

/// Writes looked back over for one a later write makes redundant: a drag
/// writes the same few paths each frame, so what it would drop is near
/// the end.
const LOOK_BACK: usize = 16;

impl DataPatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.writes.len()
    }

    pub fn writes(&self) -> impl Iterator<Item = &PatchWrite> {
        self.writes.iter()
    }

    pub fn set(&mut self, path: Vec<PathStep>, value: Value) {
        self.push(PatchWrite::Set { path, value });
    }

    pub fn remove(&mut self, path: Vec<PathStep>) {
        self.push(PatchWrite::Remove { path });
    }

    pub fn insert(&mut self, list: Vec<PathStep>, after: Option<String>, value: Value) {
        self.push(PatchWrite::Insert { list, after, value });
    }

    /// Append `write`, folding it into an earlier write it makes
    /// redundant: a set of what an earlier set or insert wrote, with
    /// nothing between them touching it, takes that write's place.
    pub fn push(&mut self, write: PatchWrite) {
        let PatchWrite::Set { path, value } = write else {
            self.writes.push_back(write);
            return;
        };
        if let Some(at) = self.touching_back(&path) {
            match &mut self.writes[at] {
                PatchWrite::Set { path: earlier, .. } if *earlier == path => {
                    self.writes.remove(at);
                }
                PatchWrite::Insert {
                    list,
                    value: inserted,
                    ..
                } if path.len() == list.len() + 1
                    && path.starts_with(list)
                    && item_id(&value) == item_id(inserted) =>
                {
                    *inserted = value;
                    return;
                }
                _ => {}
            }
        }
        self.writes.push_back(PatchWrite::Set { path, value });
    }

    /// `earlier` and then this patch, joined into one: what applying
    /// `earlier` then `self` does. A set in `earlier` that a set of
    /// `self` overwrites before anything else touches it is dropped.
    pub fn after(&mut self, earlier: DataPatch) {
        for write in earlier.writes.into_iter().rev() {
            let overwritten = match &write {
                PatchWrite::Set { path: target, .. } => self.touching_front(target).is_some_and(
                    |at| matches!(&self.writes[at], PatchWrite::Set { path, .. } if path == target),
                ),
                _ => false,
            };
            if !overwritten {
                self.writes.push_front(write);
            }
        }
    }

    /// This patch and then `later`, joined into one.
    pub fn then(&mut self, later: DataPatch) {
        for write in later.writes {
            self.push(write);
        }
    }

    /// The latest of the last few writes that touches `path`.
    fn touching_back(&self, path: &[PathStep]) -> Option<usize> {
        let len = self.writes.len();
        (len.saturating_sub(LOOK_BACK)..len)
            .rev()
            .find(|&at| self.writes[at].target().is_none_or(|t| related(&t, path)))
    }

    /// The first of the first few writes that touches `path`.
    fn touching_front(&self, path: &[PathStep]) -> Option<usize> {
        (0..self.writes.len().min(LOOK_BACK))
            .find(|&at| self.writes[at].target().is_none_or(|t| related(&t, path)))
    }

    /// Apply every write to `data`, in order, answering the patch that
    /// undoes them. When one does not apply, the ones before it are put
    /// back and `data` is left as it was.
    pub fn apply(&self, data: &mut Value) -> Result<DataPatch, PatchError> {
        let mut undo = DataPatch::new();
        for (at, write) in self.writes.iter().enumerate() {
            match apply_write(data, write) {
                Ok(inverse) => undo.writes.push_front(inverse),
                Err(reason) => {
                    for inverse in &undo.writes {
                        apply_write(data, inverse).expect("an inverse applies where it was made");
                    }
                    return Err(PatchError { write: at, reason });
                }
            }
        }
        Ok(undo)
    }
}

/// Whether one path leads into the other (or they are the same).
fn related(a: &[PathStep], b: &[PathStep]) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// The id of a list's item: its `id`, or its one variant's `id`.
pub fn item_id(item: &Value) -> Option<&str> {
    let map = item.as_object()?;
    if let Some(id) = map.get("id") {
        return id.as_str();
    }
    match map.len() {
        1 => map.values().next()?.get("id")?.as_str(),
        _ => None,
    }
}

/// Where in `list` the item with id `id` is: looked for from the end,
/// where an edit most often is.
fn position(list: &[Value], id: &str) -> Option<usize> {
    list.iter().rposition(|item| item_id(item) == Some(id))
}

fn walk<'a>(mut at: &'a mut Value, path: &[PathStep]) -> Result<&'a mut Value, &'static str> {
    for step in path {
        at = match step {
            PathStep::Key(key) => at
                .as_object_mut()
                .ok_or("a key into what is not an object")?
                .get_mut(key)
                .ok_or("a key that is missing")?,
            PathStep::Item { id } => {
                let list = at.as_array_mut().ok_or("an item of what is not a list")?;
                let found = position(list, id).ok_or("an item that is missing")?;
                &mut list[found]
            }
        };
    }
    Ok(at)
}

/// Apply one write, answering the write that undoes it.
fn apply_write(data: &mut Value, write: &PatchWrite) -> Result<PatchWrite, &'static str> {
    match write {
        PatchWrite::Set { path, value } => {
            let (last, parent) = path.split_last().ok_or("a set of the whole payload")?;
            let parent = walk(data, parent)?;
            match last {
                PathStep::Key(key) => {
                    let map = parent
                        .as_object_mut()
                        .ok_or("a key into what is not an object")?;
                    Ok(match map.insert(key.clone(), value.clone()) {
                        Some(old) => PatchWrite::Set {
                            path: path.clone(),
                            value: old,
                        },
                        None => PatchWrite::Remove { path: path.clone() },
                    })
                }
                PathStep::Item { id } => {
                    if item_id(value) != Some(id) {
                        return Err("an item set to one with another id");
                    }
                    let list = parent
                        .as_array_mut()
                        .ok_or("an item of what is not a list")?;
                    let found = position(list, id).ok_or("an item that is missing")?;
                    let old = std::mem::replace(&mut list[found], value.clone());
                    Ok(PatchWrite::Set {
                        path: path.clone(),
                        value: old,
                    })
                }
            }
        }
        PatchWrite::Remove { path } => {
            let (last, parent_path) = path.split_last().ok_or("a removal of the whole payload")?;
            let parent = walk(data, parent_path)?;
            match last {
                PathStep::Key(key) => {
                    let map = parent
                        .as_object_mut()
                        .ok_or("a key into what is not an object")?;
                    let old = map.remove(key).ok_or("a key that is missing")?;
                    Ok(PatchWrite::Set {
                        path: path.clone(),
                        value: old,
                    })
                }
                PathStep::Item { id } => {
                    let list = parent
                        .as_array_mut()
                        .ok_or("an item of what is not a list")?;
                    let found = position(list, id).ok_or("an item that is missing")?;
                    let after = found
                        .checked_sub(1)
                        .map(|before| item_id(&list[before]).map(str::to_owned))
                        .map(|id| id.ok_or("an item without an id before the one removed"))
                        .transpose()?;
                    let old = list.remove(found);
                    Ok(PatchWrite::Insert {
                        list: parent_path.to_vec(),
                        after,
                        value: old,
                    })
                }
            }
        }
        PatchWrite::Insert { list, after, value } => {
            let id = item_id(value).ok_or("an item without an id")?;
            let items = walk(data, list)?
                .as_array_mut()
                .ok_or("an insert into what is not a list")?;
            let at = match after {
                None => 0,
                Some(before) => {
                    position(items, before).ok_or("an item to follow that is missing")? + 1
                }
            };
            let mut path = list.clone();
            path.push(PathStep::item(id));
            items.insert(at, value.clone());
            Ok(PatchWrite::Remove { path })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(k: &str) -> PathStep {
        PathStep::key(k)
    }

    fn list() -> Vec<PathStep> {
        vec![key("sketch"), key("geometry")]
    }

    fn item(id: &str) -> Vec<PathStep> {
        let mut path = list();
        path.push(PathStep::item(id));
        path
    }

    fn data() -> Value {
        json!({
            "name": "s",
            "sketch": {
                "geometry": [
                    {"Point": {"id": "a", "x": 0.0}},
                    {"Line": {"id": "b", "x": 1.0}},
                    {"Point": {"id": "c", "x": 2.0}},
                ],
                "constraints": [{"id": "k", "kind": "Fixed"}],
            },
        })
    }

    /// Every kind of write applies in order and the answered patch puts
    /// the data back exactly; applying the patch again after that lands
    /// where the first apply did.
    #[test]
    fn a_patch_and_its_inverse_round_trip() {
        let mut patch = DataPatch::new();
        patch.remove(item("b"));
        patch.remove(item("c"));
        patch.insert(list(), Some("a".into()), json!({"Circle": {"id": "d"}}));
        patch.insert(list(), None, json!({"Point": {"id": "e"}}));
        patch.set(item("a"), json!({"Point": {"id": "a", "x": 5.0}}));
        patch.set(vec![key("name")], json!("t"));
        patch.set(vec![key("plane")], json!([0, 0, 1]));
        patch.remove(vec![key("sketch"), key("constraints"), PathStep::item("k")]);

        let before = data();
        let mut edited = before.clone();
        let undo = patch.apply(&mut edited).expect("applies");
        assert_eq!(
            edited["sketch"]["geometry"],
            json!([
                {"Point": {"id": "e"}},
                {"Point": {"id": "a", "x": 5.0}},
                {"Circle": {"id": "d"}},
            ])
        );
        assert_eq!(edited["name"], "t");
        assert_eq!(edited["plane"], json!([0, 0, 1]));
        assert_eq!(edited["sketch"]["constraints"], json!([]));

        let after = edited.clone();
        let redo = undo.apply(&mut edited).expect("the inverse applies");
        assert_eq!(edited, before, "undone exactly");
        redo.apply(&mut edited).expect("the redo applies");
        assert_eq!(edited, after, "redone exactly");
    }

    /// A write whose item is gone (a peer removed it) leaves the data as
    /// it found it, the writes before it put back.
    #[test]
    fn a_patch_that_does_not_fit_changes_nothing() {
        let mut patch = DataPatch::new();
        patch.set(vec![key("name")], json!("t"));
        patch.remove(item("a"));
        patch.set(item("gone"), json!({"Point": {"id": "gone"}}));
        let mut edited = data();
        let error = patch.apply(&mut edited).expect_err("an item is missing");
        assert_eq!(error.write, 2);
        assert_eq!(edited, data());

        for bad in [
            PatchWrite::Insert {
                list: list(),
                after: Some("gone".into()),
                value: json!({"Point": {"id": "z"}}),
            },
            PatchWrite::Set {
                path: item("a"),
                value: json!({"Point": {"id": "z"}}),
            },
            PatchWrite::Remove {
                path: vec![key("missing")],
            },
        ] {
            let mut patch = DataPatch::new();
            patch.push(bad);
            assert!(patch.apply(&mut edited).is_err());
            assert_eq!(edited, data());
        }
    }

    /// Patches joined as a drag joins them stay as small as the last
    /// frame, and the joined inverse goes back to where the first began.
    #[test]
    fn joined_patches_keep_the_last_write_and_the_first_inverse() {
        let start = data();
        let mut doc = start.clone();
        let mut forward = DataPatch::new();
        let mut backward = DataPatch::new();
        for x in 0..50 {
            let mut frame = DataPatch::new();
            frame.set(item("b"), json!({"Line": {"id": "b", "x": f64::from(x)}}));
            let undo = frame.apply(&mut doc).expect("applies");
            forward.then(frame);
            backward.after(undo);
        }
        assert_eq!(forward.len(), 1, "one write forward");
        assert_eq!(backward.len(), 1, "one write back");
        let mut replay = start.clone();
        forward.apply(&mut replay).expect("applies");
        assert_eq!(replay, doc);
        backward.apply(&mut doc).expect("applies");
        assert_eq!(doc, start);

        // An item put in and then moved is one insert of where it ended.
        let mut added = DataPatch::new();
        added.insert(
            list(),
            Some("c".into()),
            json!({"Point": {"id": "n", "x": 0.0}}),
        );
        added.set(item("n"), json!({"Point": {"id": "n", "x": 9.0}}));
        assert_eq!(added.len(), 1);
    }

    /// The wire form: a list of writes, each named, a path's item step an
    /// object with its id.
    #[test]
    fn a_patch_reads_back_from_its_json() {
        let mut patch = DataPatch::new();
        patch.set(item("a"), json!({"Point": {"id": "a"}}));
        patch.insert(list(), None, json!({"Point": {"id": "e"}}));
        let text = serde_json::to_string(&patch).expect("writes");
        assert!(
            text.starts_with(r#"[{"set":{"path":["sketch","geometry",{"id":"a"}]"#),
            "{text}"
        );
        let back: DataPatch = serde_json::from_str(&text).expect("reads");
        assert_eq!(back, patch);
    }
}
