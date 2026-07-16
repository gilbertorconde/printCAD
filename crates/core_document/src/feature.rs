//! Feature tree and parametric model structures.
//!
//! This module provides a generic, extensible feature tree that allows workbenches
//! to define their own feature types without modifying the core document structure.

use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;
use uuid::Uuid;

use crate::{DocumentResult, WorkbenchId};

/// Unique identifier for a feature in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FeatureId(pub Uuid);

impl FeatureId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for FeatureId {
    fn default() -> Self {
        Self::new()
    }
}

/// Unique identifier for a body in the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BodyId(pub Uuid);

impl BodyId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BodyId {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for workbench-specific feature types.
///
/// Workbenches implement this trait to define their own feature types that can be
/// stored in the document's feature tree. The document stores features as type-erased
/// JSON, and workbenches handle serialization/deserialization.
pub trait WorkbenchFeature: Send + Sync {
    /// The workbench this feature belongs to.
    fn workbench_id() -> WorkbenchId
    where
        Self: Sized;

    /// Serialize this feature to JSON.
    fn to_json(&self) -> serde_json::Value;

    /// Deserialize from JSON.
    fn from_json(value: &serde_json::Value) -> DocumentResult<Self>
    where
        Self: Sized;

    /// Get dependencies (other feature IDs this feature depends on).
    fn dependencies(&self) -> Vec<FeatureId>;

    /// Get the feature name.
    fn name(&self) -> &str;
}

/// A feature node in the tree (type-erased).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureNode {
    pub id: FeatureId,
    pub workbench_id: WorkbenchId,
    pub name: String,
    /// Owning body for this feature (if any). Used for tree hierarchy / grouping.
    #[serde(default)]
    pub body: Option<BodyId>,
    pub visible: bool,
    pub suppressed: bool,
    pub dirty: bool,
    pub created_at: i64,
    /// Monotonic insertion sequence within the document. THE ordering key
    /// for build histories — `created_at` has millisecond resolution and
    /// ties would otherwise order nondeterministically.
    #[serde(default)]
    pub seq: u64,
    /// Last recompute error for this feature. Derived state: set by the
    /// recompute driver, never persisted.
    #[serde(skip)]
    pub error: Option<String>,
    /// Type-erased feature data (serialized JSON)
    pub data: serde_json::Value,
}

impl FeatureNode {
    pub fn new<F: WorkbenchFeature>(id: FeatureId, feature: &F) -> Self {
        Self {
            id,
            workbench_id: F::workbench_id(),
            name: feature.name().to_string(),
            body: None,
            visible: true,
            suppressed: false,
            dirty: false,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            seq: 0,
            error: None,
            data: feature.to_json(),
        }
    }
}

/// Directed acyclic graph representing the feature tree.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureTree {
    /// Root features (no dependencies).
    roots: Vec<FeatureId>,
    /// All features indexed by ID (type-erased).
    features: HashMap<FeatureId, FeatureNode>,
    /// Dependency graph: feature -> list of dependencies.
    dependencies: HashMap<FeatureId, Vec<FeatureId>>,
    /// Reverse dependencies: feature -> list of dependents.
    dependents: HashMap<FeatureId, Vec<FeatureId>>,
}

impl FeatureTree {
    /// Create a new empty feature tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Next insertion sequence number (max existing + 1).
    pub fn next_seq(&self) -> u64 {
        self.features
            .values()
            .map(|n| n.seq)
            .max()
            .map_or(0, |m| m + 1)
    }

    /// Add a feature node to the tree.
    pub fn add_node(&mut self, node: FeatureNode) -> FeatureId {
        let id = node.id;

        // If feature has no dependencies, it's a root
        if !self.dependencies.contains_key(&id) {
            self.roots.push(id);
        }

        self.features.insert(id, node);
        id
    }

    /// Get a feature node by ID.
    pub fn get_node(&self, id: FeatureId) -> Option<&FeatureNode> {
        self.features.get(&id)
    }

    /// Get a mutable feature node by ID.
    pub fn get_node_mut(&mut self, id: FeatureId) -> Option<&mut FeatureNode> {
        self.features.get_mut(&id)
    }

    /// Add a dependency: `dependent` depends on `dependency`.
    pub fn add_dependency(&mut self, dependent: FeatureId, dependency: FeatureId) {
        // Add to dependencies
        self.dependencies
            .entry(dependent)
            .or_default()
            .push(dependency);

        // Add to reverse dependencies
        self.dependents
            .entry(dependency)
            .or_default()
            .push(dependent);

        // Remove from roots if it was a root
        self.roots.retain(|&id| id != dependent);
    }

    /// Get all dependencies of a feature.
    pub fn dependencies(&self, feature: FeatureId) -> Vec<FeatureId> {
        self.dependencies.get(&feature).cloned().unwrap_or_default()
    }

    /// Get all features that depend on this one.
    pub fn dependents(&self, feature: FeatureId) -> Vec<FeatureId> {
        self.dependents.get(&feature).cloned().unwrap_or_default()
    }

    /// Mark a feature and all its dependents as dirty.
    pub fn mark_dirty(&mut self, feature: FeatureId) {
        let mut to_mark = VecDeque::new();
        to_mark.push_back(feature);

        while let Some(id) = to_mark.pop_front() {
            if let Some(node) = self.features.get_mut(&id) {
                if !node.dirty {
                    node.dirty = true;
                    // Add all dependents to the queue
                    to_mark.extend(self.dependents(id));
                }
            }
        }
    }

    /// Get all dirty features.
    pub fn dirty_features(&self) -> Vec<FeatureId> {
        self.features
            .iter()
            .filter(|(_, node)| node.dirty)
            .map(|(id, _)| *id)
            .collect()
    }

    /// Get recomputation order (topological sort) for dirty features.
    pub fn recompute_order(&self, dirty_features: &[FeatureId]) -> Vec<FeatureId> {
        if dirty_features.is_empty() {
            return Vec::new();
        }

        let dirty_set: HashSet<FeatureId> = dirty_features.iter().copied().collect();
        let mut in_degree: HashMap<FeatureId, usize> = HashMap::new();
        let mut queue = VecDeque::new();
        let mut result = Vec::new();

        // Calculate in-degrees for dirty features and their dependents
        for &feature_id in dirty_features {
            in_degree.insert(feature_id, 0);
            for dep in self.dependencies(feature_id) {
                if dirty_set.contains(&dep) {
                    *in_degree.entry(feature_id).or_insert(0) += 1;
                }
            }
        }

        // Add features with no dependencies to queue
        for &feature_id in dirty_features {
            if in_degree.get(&feature_id).copied().unwrap_or(0) == 0 {
                queue.push_back(feature_id);
            }
        }

        // Topological sort
        while let Some(feature_id) = queue.pop_front() {
            result.push(feature_id);

            for dependent in self.dependents(feature_id) {
                if dirty_set.contains(&dependent) {
                    let deg = in_degree.entry(dependent).or_insert(0);
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dependent);
                    }
                }
            }
        }

        result
    }

    /// Replace a node's dependency edges (e.g. a Pad re-targeted onto a
    /// different sketch). Fixes both edge directions and root bookkeeping.
    pub fn set_dependencies(&mut self, id: FeatureId, new_deps: Vec<FeatureId>) {
        if !self.features.contains_key(&id) {
            return;
        }
        // Drop old reverse edges.
        for deps in self.dependents.values_mut() {
            deps.retain(|&d| d != id);
        }
        self.dependencies.remove(&id);
        self.roots.retain(|&r| r != id);
        if new_deps.is_empty() {
            self.roots.push(id);
        } else {
            for dep in new_deps {
                self.add_dependency(id, dep);
            }
        }
    }

    /// Remove a node and every dependency-graph edge that references it.
    /// Returns false when the id is unknown.
    pub fn remove_node(&mut self, id: FeatureId) -> bool {
        if self.features.remove(&id).is_none() {
            return false;
        }
        self.roots.retain(|&r| r != id);
        self.dependencies.remove(&id);
        self.dependents.remove(&id);
        for deps in self.dependencies.values_mut() {
            deps.retain(|&d| d != id);
        }
        for deps in self.dependents.values_mut() {
            deps.retain(|&d| d != id);
        }
        true
    }

    /// Get all root features.
    pub fn roots(&self) -> &[FeatureId] {
        &self.roots
    }

    /// Get all feature nodes.
    pub fn all_nodes(&self) -> impl Iterator<Item = (&FeatureId, &FeatureNode)> {
        self.features.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_node(id: FeatureId) -> FeatureNode {
        FeatureNode {
            id,
            workbench_id: WorkbenchId::new("wb.test"),
            name: "test".into(),
            body: None,
            visible: true,
            suppressed: false,
            dirty: false,
            created_at: 0,
            seq: 0,
            error: None,
            data: serde_json::Value::Null,
        }
    }

    /// Diamond DAG: a ← b, a ← c, b ← d, c ← d (d depends on b and c).
    /// `add_dependency` is called before `add_node` for non-roots so root
    /// bookkeeping in `add_node` sees the dependency entry (the tree's
    /// root check is order-sensitive; this is the supported call order,
    /// matching `Document::add_feature`).
    fn diamond() -> (FeatureTree, FeatureId, FeatureId, FeatureId, FeatureId) {
        let (a, b, c, d) = (
            FeatureId::new(),
            FeatureId::new(),
            FeatureId::new(),
            FeatureId::new(),
        );
        let mut tree = FeatureTree::new();
        tree.add_node(test_node(a));
        for (dependent, dependency) in [(b, a), (c, a), (d, b), (d, c)] {
            tree.add_dependency(dependent, dependency);
        }
        tree.add_node(test_node(b));
        tree.add_node(test_node(c));
        tree.add_node(test_node(d));
        (tree, a, b, c, d)
    }

    #[test]
    fn diamond_has_single_root() {
        let (tree, a, ..) = diamond();
        assert_eq!(tree.roots(), &[a]);
    }

    #[test]
    fn mark_dirty_propagates_to_all_dependents() {
        let (mut tree, a, b, c, d) = diamond();
        tree.mark_dirty(a);
        let dirty: HashSet<FeatureId> = tree.dirty_features().into_iter().collect();
        assert_eq!(dirty, HashSet::from([a, b, c, d]));
    }

    #[test]
    fn mark_dirty_on_branch_leaves_sibling_clean() {
        let (mut tree, _a, b, c, d) = diamond();
        tree.mark_dirty(b);
        let dirty: HashSet<FeatureId> = tree.dirty_features().into_iter().collect();
        assert_eq!(dirty, HashSet::from([b, d]));
        assert!(!tree.get_node(c).unwrap().dirty);
    }

    #[test]
    fn mark_dirty_is_idempotent() {
        let (mut tree, a, ..) = diamond();
        tree.mark_dirty(a);
        tree.mark_dirty(a);
        assert_eq!(tree.dirty_features().len(), 4);
    }

    #[test]
    fn recompute_order_respects_dependencies() {
        let (mut tree, a, b, c, d) = diamond();
        tree.mark_dirty(a);
        let order = tree.recompute_order(&tree.dirty_features());
        assert_eq!(order.len(), 4);
        let pos = |id: FeatureId| order.iter().position(|&x| x == id).unwrap();
        assert!(pos(a) < pos(b));
        assert!(pos(a) < pos(c));
        assert!(pos(b) < pos(d));
        assert!(pos(c) < pos(d));
    }

    #[test]
    fn recompute_order_ignores_dependencies_outside_dirty_set() {
        let (mut tree, _a, b, _c, d) = diamond();
        tree.mark_dirty(b);
        let order = tree.recompute_order(&tree.dirty_features());
        // `a` and `c` are clean: only the dirty chain is ordered, with the
        // clean dependency `c` of `d` treated as already up to date.
        assert_eq!(order, vec![b, d]);
    }

    #[test]
    fn recompute_order_empty_input() {
        let (tree, ..) = diamond();
        assert!(tree.recompute_order(&[]).is_empty());
    }

    /// Pins current behaviour: members of a dependency cycle are silently
    /// dropped from the recompute order (Kahn's algorithm never reaches
    /// in-degree 0 for them). If cycles should become a hard error, this
    /// test is the place that documents the change.
    #[test]
    fn recompute_order_drops_cycle_members() {
        let (x, y) = (FeatureId::new(), FeatureId::new());
        let mut tree = FeatureTree::new();
        tree.add_node(test_node(x));
        tree.add_node(test_node(y));
        tree.add_dependency(x, y);
        tree.add_dependency(y, x);
        tree.mark_dirty(x);
        let order = tree.recompute_order(&tree.dirty_features());
        assert!(order.is_empty(), "cycle members are dropped, got {order:?}");
    }

    #[test]
    fn add_dependency_removes_dependent_from_roots() {
        let (x, y) = (FeatureId::new(), FeatureId::new());
        let mut tree = FeatureTree::new();
        tree.add_node(test_node(x));
        tree.add_node(test_node(y));
        assert_eq!(tree.roots().len(), 2);
        tree.add_dependency(y, x);
        assert_eq!(tree.roots(), &[x]);
        assert_eq!(tree.dependencies(y), vec![x]);
        assert_eq!(tree.dependents(x), vec![y]);
    }
}

/// Errors that can occur when working with features.
#[derive(Debug, Error)]
pub enum FeatureError {
    #[error("feature deserialization failed: {0}")]
    Deserialization(String),
    #[error("feature not found: {0:?}")]
    NotFound(FeatureId),
    #[error("invalid workbench: expected {expected:?}, got {got:?}")]
    InvalidWorkbench {
        expected: WorkbenchId,
        got: WorkbenchId,
    },
}
