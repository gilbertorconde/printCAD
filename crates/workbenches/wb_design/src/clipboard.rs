//! Copying features: a feature with the sketches and datums of its body it
//! is built from, taken as data and made again in any body, each copy
//! reading the copies of what the original read.

use std::collections::BTreeMap;

use core_document::{BodyId, Document, FeatureId, FeatureOrigin, WorkbenchId};
use serde_json::Value;

/// One feature as copied.
#[derive(Debug, Clone)]
struct Copied {
    id: FeatureId,
    kind: WorkbenchId,
    name: String,
    data: Value,
    deps: Vec<FeatureId>,
    formulas: BTreeMap<String, String>,
    visible: bool,
}

/// Features taken to be made again, in history order, the one copied last.
#[derive(Debug, Clone, Default)]
pub struct Clipboard {
    features: Vec<Copied>,
}

impl Clipboard {
    /// `id` with the sketches and datums of its own body it reads, however
    /// far back. `None` for a feature of no body.
    pub fn copy(document: &Document, id: FeatureId) -> Option<Self> {
        let body = document.get_feature_meta(id)?.body?;
        let mut ids = vec![id];
        let mut i = 0;
        while i < ids.len() {
            for dep in document.feature_tree().dependencies(ids[i]) {
                let takes = document.get_feature_meta(dep).is_some_and(|n| {
                    n.body == Some(body)
                        && matches!(n.workbench_id.as_str(), "wb.sketch" | "core.datum")
                });
                if takes && !ids.contains(&dep) {
                    ids.push(dep);
                }
            }
            i += 1;
        }
        let mut features: Vec<Copied> = ids
            .into_iter()
            .filter_map(|fid| {
                let node = document.get_feature_meta(fid)?;
                Some(Copied {
                    id: fid,
                    kind: node.workbench_id.clone(),
                    name: node.name.clone(),
                    data: node.data.clone(),
                    deps: document.feature_tree().dependencies(fid),
                    formulas: node.formulas.clone(),
                    visible: node.visible,
                })
            })
            .collect();
        let seq = |fid: FeatureId| document.get_feature_meta(fid).map(|n| n.seq);
        features.sort_by_key(|c| (seq(c.id), c.id));
        Some(Self { features })
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    /// The features copied, the one asked for last.
    pub fn ids(&self) -> Vec<FeatureId> {
        self.features.iter().map(|c| c.id).collect()
    }

    /// Make the features again in `body`, at its tip, each named after its
    /// original as a new feature is. Returns the new features, the copy of
    /// the one asked for last.
    pub fn paste(&self, document: &mut Document, body: BodyId) -> Vec<FeatureId> {
        let mut map: Vec<(String, String)> = Vec::new();
        let mut made = Vec::new();
        for copied in &self.features {
            let mut data = copied.data.clone();
            for (old, new) in &map {
                replace_ids(&mut data, old, new);
            }
            let deps: Vec<FeatureId> = copied
                .deps
                .iter()
                .map(|dep| {
                    made.iter()
                        .zip(&self.features)
                        .find(|(_, c)| c.id == *dep)
                        .map_or(*dep, |(new, _)| *new)
                })
                .collect();
            let base = base_name(&copied.name);
            let name = document.unused_object_name(base);
            let id = document.add_feature_of_kind(
                copied.kind.clone(),
                name,
                Some(body),
                deps,
                data,
                FeatureOrigin::default(),
            );
            for (key, formula) in &copied.formulas {
                let _ = document.set_feature_formula(id, key.clone(), Some(formula.clone()));
            }
            if !copied.visible {
                document.set_feature_visible(id, false);
            }
            map.push((copied.id.0.to_string(), id.0.to_string()));
            made.push(id);
        }
        crate::invalidate_body(document, body);
        made
    }
}

/// A name without the `_n` a copy's name ends in.
fn base_name(name: &str) -> &str {
    match name.rsplit_once('_') {
        Some((base, n)) if !base.is_empty() && n.parse::<u32>().is_ok() => base,
        _ => name,
    }
}

/// Every string in `value` that is `old` becomes `new`.
fn replace_ids(value: &mut Value, old: &str, new: &str) {
    match value {
        Value::String(s) if s == old => *s = new.to_string(),
        Value::Array(items) => items.iter_mut().for_each(|v| replace_ids(v, old, new)),
        Value::Object(map) => map.values_mut().for_each(|v| replace_ids(v, old, new)),
        _ => {}
    }
}

/// Remove the feature copied, then each feature copied with it that
/// nothing else reads any more.
pub fn cut(document: &mut Document, clipboard: &Clipboard) {
    let mut ids = clipboard.ids();
    ids.reverse();
    for id in ids {
        if document.feature_tree().dependents(id).is_empty() {
            crate::delete_feature(document, id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_names_drop_their_number() {
        assert_eq!(base_name("Pad_3"), "Pad");
        assert_eq!(base_name("Pad"), "Pad");
        assert_eq!(base_name("Pad_x"), "Pad_x");
        assert_eq!(base_name("_2"), "_2");
    }
}
