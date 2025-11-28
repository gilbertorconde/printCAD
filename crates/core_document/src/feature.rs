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
    pub visible: bool,
    pub suppressed: bool,
    pub dirty: bool,
    pub created_at: i64,
    /// Type-erased feature data (serialized JSON)
    pub data: serde_json::Value,
}

impl FeatureNode {
    pub fn new<F: WorkbenchFeature>(id: FeatureId, feature: &F) -> Self {
        Self {
            id,
            workbench_id: F::workbench_id(),
            name: feature.name().to_string(),
            visible: true,
            suppressed: false,
            dirty: false,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
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
            .or_insert_with(Vec::new)
            .push(dependency);

        // Add to reverse dependencies
        self.dependents
            .entry(dependency)
            .or_insert_with(Vec::new)
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

    /// Get all root features.
    pub fn roots(&self) -> &[FeatureId] {
        &self.roots
    }

    /// Get all feature nodes.
    pub fn all_nodes(&self) -> impl Iterator<Item = (&FeatureId, &FeatureNode)> {
        self.features.iter()
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
