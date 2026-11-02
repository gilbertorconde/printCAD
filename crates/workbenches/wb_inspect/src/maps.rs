//! Comparisons measured away from the window: each comparison's distances
//! kept with the key of what they were measured from, measured again on a
//! thread of their own when that changes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use core_document::FeatureId;
use kernel_api::KernelQueries;

use crate::compare::{self, Comparison, Inputs};

#[derive(Default)]
struct State {
    measured: HashMap<FeatureId, (u64, Arc<[f32]>)>,
    /// What each comparison's distances say, by the key they were measured
    /// from and the comparison's settings.
    read: HashMap<FeatureId, (u64, compare::Reading)>,
    running: HashSet<FeatureId>,
    failed: HashMap<FeatureId, (u64, String)>,
}

/// Every comparison's latest distances.
#[derive(Default, Clone)]
pub struct Maps {
    state: Arc<Mutex<State>>,
}

impl Maps {
    /// `id`'s distances when they were measured from what `key` names.
    pub fn values(&self, id: FeatureId, key: u64) -> Option<Arc<[f32]>> {
        let state = self.state.lock().ok()?;
        state
            .measured
            .get(&id)
            .filter(|(k, _)| *k == key)
            .map(|(_, v)| v.clone())
    }

    /// Why `id` could not be measured from what `key` names, if it could not.
    pub fn failure(&self, id: FeatureId, key: u64) -> Option<String> {
        let state = self.state.lock().ok()?;
        state
            .failed
            .get(&id)
            .filter(|(k, _)| *k == key)
            .map(|(_, why)| why.clone())
    }

    /// Measure `id` on a thread unless it is measured, failed or running for
    /// these inputs already.
    pub fn request(
        &self,
        id: FeatureId,
        inputs: Inputs,
        c: Comparison,
        kernel: &'static dyn KernelQueries,
    ) {
        {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            let key = inputs.key;
            if state.running.contains(&id)
                || state.measured.get(&id).is_some_and(|(k, _)| *k == key)
                || state.failed.get(&id).is_some_and(|(k, _)| *k == key)
            {
                return;
            }
            state.running.insert(id);
        }
        let state = self.state.clone();
        std::thread::spawn(move || {
            let answer = compare::measure(kernel, &inputs, &c);
            if let Ok(mut state) = state.lock() {
                state.running.remove(&id);
                match answer {
                    Ok(values) => {
                        state.failed.remove(&id);
                        state.measured.insert(id, (inputs.key, values));
                    }
                    Err(why) => {
                        state.failed.insert(id, (inputs.key, why));
                    }
                }
            }
        });
    }

    /// Keep `values` as `id`'s, measured from what `key` names.
    pub fn store(&self, id: FeatureId, key: u64, values: Arc<[f32]>) {
        if let Ok(mut state) = self.state.lock() {
            state.measured.insert(id, (key, values));
        }
    }

    /// What `id`'s distances say as `c` reads them, worked out once per
    /// measurement and settings.
    pub fn reading(&self, id: FeatureId, key: u64, c: &Comparison) -> Option<compare::Reading> {
        let values = self.values(id, key)?;
        let stamp = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut h);
            format!("{:?}{:?}", c.range, c.tolerance).hash(&mut h);
            h.finish()
        };
        let mut state = self.state.lock().ok()?;
        if let Some((s, r)) = state.read.get(&id)
            && *s == stamp
        {
            return Some(r.clone());
        }
        let r = compare::read(&values, c);
        state.read.insert(id, (stamp, r.clone()));
        Some(r)
    }

    /// Whether any comparison is being measured.
    pub fn busy(&self) -> bool {
        self.state.lock().is_ok_and(|s| !s.running.is_empty())
    }
}
