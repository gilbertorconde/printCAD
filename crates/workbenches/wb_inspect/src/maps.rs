//! Maps measured away from the window: each map's values
//! kept with the key of what they were measured from, measured again on a
//! thread of their own when that changes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use core_document::FeatureId;
use kernel_api::{KernelQueries, VolumeRegion};

use crate::compare::{self, Comparison, Inputs};

/// Volume errors as read, or why they could not be.
type Volumes = Result<Arc<Vec<VolumeRegion>>, String>;

#[derive(Default)]
struct State {
    measured: HashMap<FeatureId, (u64, Arc<[f32]>)>,
    /// What each comparison's distances say, by the key they were measured
    /// from and the comparison's settings.
    read: HashMap<FeatureId, (u64, compare::Reading)>,
    /// Each comparison's hot spots, by measurement and tolerance.
    spots: HashMap<FeatureId, (u64, Arc<Vec<crate::hotspots::HotSpot>>)>,
    running: HashSet<FeatureId>,
    failed: HashMap<FeatureId, (u64, String)>,
    /// Each comparison's volume errors, by measurement and cell, or why
    /// they could not be read.
    volumes: HashMap<FeatureId, (u64, Volumes)>,
    reading_volumes: HashSet<FeatureId>,
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
        let key = inputs.key;
        self.run(id, key, move || compare::measure(kernel, &inputs, &c));
    }

    /// Run `job` for `id` on a thread unless its answer for `key` is in,
    /// failed or on its way already.
    pub fn run(
        &self,
        id: FeatureId,
        key: u64,
        job: impl FnOnce() -> Result<Arc<[f32]>, String> + Send + 'static,
    ) {
        {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
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
            let answer = job();
            if let Ok(mut state) = state.lock() {
                state.running.remove(&id);
                match answer {
                    Ok(values) => {
                        state.failed.remove(&id);
                        state.measured.insert(id, (key, values));
                    }
                    Err(why) => {
                        state.failed.insert(id, (key, why));
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

    /// `id`'s hot spots at `tolerance`, found by `find` once per measurement
    /// and tolerance.
    pub fn hot_spots(
        &self,
        id: FeatureId,
        key: u64,
        tolerance: f64,
        find: impl FnOnce() -> Option<Vec<crate::hotspots::HotSpot>>,
    ) -> Option<Arc<Vec<crate::hotspots::HotSpot>>> {
        let stamp = key ^ tolerance.to_bits().rotate_left(17);
        if let Ok(state) = self.state.lock()
            && let Some((s, spots)) = state.spots.get(&id)
            && *s == stamp
        {
            return Some(spots.clone());
        }
        let spots = Arc::new(find()?);
        if let Ok(mut state) = self.state.lock() {
            state.spots.insert(id, (stamp, spots.clone()));
        }
        Some(spots)
    }

    /// The stamp volume errors read from `key` on cells `cell` wide are
    /// kept by.
    fn volume_stamp(key: u64, cell: f64) -> u64 {
        key ^ cell.to_bits().rotate_left(29)
    }

    /// `id`'s volume errors read from `key` on cells `cell` wide, or why
    /// they could not be, once they are in.
    pub fn volumes(&self, id: FeatureId, key: u64, cell: f64) -> Option<Volumes> {
        let stamp = Self::volume_stamp(key, cell);
        let state = self.state.lock().ok()?;
        state
            .volumes
            .get(&id)
            .filter(|(s, _)| *s == stamp)
            .map(|(_, r)| r.clone())
    }

    /// Keep `regions` as `id`'s volume errors from `key` on cells `cell`
    /// wide.
    pub fn store_volumes(
        &self,
        id: FeatureId,
        key: u64,
        cell: f64,
        regions: Arc<Vec<VolumeRegion>>,
    ) {
        if let Ok(mut state) = self.state.lock() {
            let stamp = Self::volume_stamp(key, cell);
            state.volumes.insert(id, (stamp, Ok(regions)));
        }
    }

    /// Read `id`'s volume errors on a thread unless they are in or on
    /// their way for these inputs and cell.
    pub fn request_volumes(
        &self,
        id: FeatureId,
        inputs: Inputs,
        cell: f64,
        kernel: &'static dyn KernelQueries,
    ) {
        let stamp = Self::volume_stamp(inputs.key, cell);
        {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.reading_volumes.contains(&id)
                || state.volumes.get(&id).is_some_and(|(s, _)| *s == stamp)
            {
                return;
            }
            state.reading_volumes.insert(id);
        }
        let state = self.state.clone();
        std::thread::spawn(move || {
            let answer = crate::volume::measure(kernel, &inputs, cell).map(Arc::new);
            if let Ok(mut state) = state.lock() {
                state.reading_volumes.remove(&id);
                state.volumes.insert(id, (stamp, answer));
            }
        });
    }

    /// Whether any comparison is being measured.
    pub fn busy(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|s| !s.running.is_empty() || !s.reading_volumes.is_empty())
    }
}
