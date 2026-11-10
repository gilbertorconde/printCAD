//! Maps measured away from the window: each map's values
//! kept with the key of what they were measured from, measured again on a
//! thread of their own when that changes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use core_document::{
    BodyId, Document, FeatureId, FeatureNode, ScalarMap, WorkbenchFeature, WorkbenchRuntimeContext,
};
use kernel_api::{KernelQueries, VolumeRegion};

use crate::compare::{self, Comparison, Inputs};

/// A feature that paints a body with a value per vertex: measured from
/// inputs taken out of the document, kept in [`Maps`] by their key, and
/// drawn through `Workbench::scalar_maps`.
pub(crate) trait Mapped: WorkbenchFeature + Clone + Send + 'static {
    /// The feature kind.
    const KIND: &'static str;
    /// A new one's name, numbered after it.
    const WORD: &'static str;
    /// What the values are measured from, out of the document.
    type Inputs: Send + 'static;

    /// The body the map paints, which the feature sits on.
    fn body(&self) -> BodyId;
    fn set_name(&mut self, name: String);
    /// What the values are measured from in `document` now.
    fn inputs(&self, document: &Document) -> Result<Self::Inputs, String>;
    /// Changes whenever the inputs or the settings measured with do.
    fn key(inputs: &Self::Inputs) -> u64;
    /// The value at every vertex of the body's mesh (`NaN` unmeasured).
    fn measure(
        &self,
        inputs: &Self::Inputs,
        kernel: Option<&'static dyn KernelQueries>,
    ) -> Result<Arc<[f32]>, String>;
    /// The map `values` paint, with its legend; `name` is the feature's.
    fn paint(
        &self,
        id: FeatureId,
        name: &str,
        inputs: &Self::Inputs,
        values: Arc<[f32]>,
        maps: &Maps,
        ctx: &WorkbenchRuntimeContext,
    ) -> Option<ScalarMap>;
}

/// Every visible `M`, the latest first.
fn latest<M: Mapped>(document: &Document) -> Vec<(FeatureId, &FeatureNode)> {
    let mut nodes: Vec<_> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == M::KIND && n.visible)
        .map(|(id, n)| (*id, n))
        .collect();
    nodes.sort_by_key(|(id, n)| (std::cmp::Reverse(n.seq), *id));
    nodes
}

/// `m` named and kept on its body, measured at once.
pub(crate) fn add<M: Mapped>(
    ctx: &mut WorkbenchRuntimeContext,
    maps: &Maps,
    mut m: M,
) -> Result<(FeatureId, M, Arc<[f32]>), String> {
    m.set_name(crate::numbered(ctx.document, M::WORD));
    let inputs = m.inputs(ctx.document)?;
    let values = m.measure(&inputs, ctx.kernel)?;
    let id = ctx
        .document
        .add_feature_in_body(m.clone(), m.name().to_string(), Some(m.body()))
        .map_err(|e| e.to_string())?;
    maps.store(id, M::key(&inputs), values.clone());
    Ok((id, m, values))
}

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

    /// The map of every visible `M` on a body `out` paints nothing on
    /// yet, the latest first; one not measured for what it reads now is
    /// measured on a thread and painted once it is in.
    pub(crate) fn paint_all<M: Mapped>(
        &self,
        ctx: &WorkbenchRuntimeContext,
        kernel: &'static dyn KernelQueries,
        out: &mut Vec<ScalarMap>,
    ) {
        let document = &*ctx.document;
        for (id, node) in latest::<M>(document) {
            let Some(m) = document.feature_as::<M>(id) else {
                continue;
            };
            let body = m.body();
            if out.iter().any(|x| x.body == body) || !document.imported_body_effective_visible(body)
            {
                continue;
            }
            let Ok(inputs) = m.inputs(document) else {
                continue;
            };
            let key = M::key(&inputs);
            let Some(values) = self.values(id, key) else {
                self.run(id, key, move || m.measure(&inputs, Some(kernel)));
                continue;
            };
            out.extend(m.paint(id, &node.name, &inputs, values, self, ctx));
        }
    }

    /// The latest visible `M` on `scan`, when there is one, with its
    /// values when they are measured for what it reads now.
    pub(crate) fn shown<M: Mapped>(
        &self,
        document: &Document,
        scan: BodyId,
    ) -> Option<Option<(FeatureId, Arc<[f32]>)>> {
        let (id, _) = latest::<M>(document)
            .into_iter()
            .find(|(_, n)| n.body == Some(scan))?;
        let values = || {
            let m = document.feature_as::<M>(id)?;
            let key = M::key(&m.inputs(document).ok()?);
            Some((id, self.values(id, key)?))
        };
        Some(values())
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
