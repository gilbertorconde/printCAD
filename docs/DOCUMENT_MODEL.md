# Document Model Design

## Overview

The `Document` is the central in-memory data structure that represents the complete state of a printCAD project. It provides a **generic, extensible API** that allows workbenches to define their own feature types and data structures.

## Core Principles

1. **Extensibility**: Workbenches define their own feature types and data structures
2. **Type Safety**: Use type-erased storage with workbench-specific deserialization
3. **Dependency Tracking**: Generic dependency graph independent of feature types
4. **Serialization**: All data must be serializable for persistence

## Core Structure

```rust
pub struct Document {
    metadata: DocumentMetadata,
    feature_tree: FeatureTree,
    bodies: HashMap<BodyId, Body>,
    /// Workbench-specific data storage (type-erased)
    workbench_storage: HashMap<WorkbenchId, WorkbenchStorage>,
    /// References to external files stored in the .prtcad archive
    assets: HashMap<Uuid, AssetReference>,
    history: Vec<DocumentRevision>,
}
```

## Feature Tree (Generic)

The feature tree is a **generic** directed acyclic graph (DAG) that doesn't know about specific feature types:

```rust
pub struct FeatureTree {
    /// Root features (no dependencies)
    roots: Vec<FeatureId>,
    /// All features indexed by ID (type-erased)
    features: HashMap<FeatureId, FeatureNode>,
    /// Dependency graph: feature -> list of dependencies
    dependencies: HashMap<FeatureId, Vec<FeatureId>>,
    /// Reverse dependencies: feature -> list of dependents
    dependents: HashMap<FeatureId, Vec<FeatureId>>,
}

/// A feature node in the tree (type-erased).
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
```

## Workbench Feature API

Workbenches define their own feature types and register them:

```rust
/// Trait for workbench-specific feature types.
pub trait WorkbenchFeature: Send + Sync {
    /// The workbench this feature belongs to.
    fn workbench_id() -> WorkbenchId;

    /// Serialize this feature to JSON.
    fn to_json(&self) -> serde_json::Value;

    /// Deserialize from JSON.
    fn from_json(value: &serde_json::Value) -> Result<Self, FeatureError>;

    /// Get dependencies (other feature IDs this feature depends on).
    fn dependencies(&self) -> Vec<FeatureId>;

    /// Get the feature name.
    fn name(&self) -> &str;
}
```

## Example: Sketch Workbench Feature

```rust
// In wb_sketch crate
pub struct SketchFeature {
    pub sketch: Sketch, // from wb_sketch::sketch
    pub plane: SketchPlane,
}

impl WorkbenchFeature for SketchFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.sketch")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap()
    }

    fn from_json(value: &serde_json::Value) -> Result<Self, FeatureError> {
        serde_json::from_value(value.clone())
            .map_err(|e| FeatureError::Deserialization(e.to_string()))
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new() // Sketches have no dependencies
    }

    fn name(&self) -> &str {
        &self.sketch.name
    }
}
```

## Example: Part Design Feature

```rust
// In wb_part crate
pub struct PadFeature {
    pub sketch: FeatureId, // reference to sketch feature
    pub distance: f32,
}

impl WorkbenchFeature for PadFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.part-design")
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        vec![self.sketch] // Depends on sketch
    }

    // ... other methods
}
```

## Workbench Storage

Workbenches can store additional data outside the feature tree:

```rust
/// Type-erased storage for workbench-specific data.
pub struct WorkbenchStorage {
    /// Workbench ID this storage belongs to
    pub workbench_id: WorkbenchId,
    /// Arbitrary JSON data (workbench-specific)
    pub data: serde_json::Value,
}
```

## Bodies

Bodies represent the final 3D geometry:

```rust
pub struct Body {
    pub id: BodyId,
    pub name: String,
    pub root_feature: FeatureId, // top-level feature that generates this body
    pub kernel_handle: Option<kernel_api::BodyHandle>, // kernel-managed geometry
    pub mesh: Option<kernel_api::TriMesh>, // cached tessellation
    pub dirty: bool,
}
```

## Workbench Data

Workbench-specific data stored separately:

```rust
pub enum WorkbenchData {
    Sketch(Vec<Sketch>), // sketches not yet in feature tree
    PartDesign(PartDesignData),
    // Future workbenches...
}
```

## Dependency Management

Features can depend on other features:

```rust
impl FeatureTree {
    /// Add a dependency: `dependent` depends on `dependency`
    pub fn add_dependency(&mut self, dependent: FeatureId, dependency: FeatureId);

    /// Get all dependencies of a feature
    pub fn dependencies(&self, feature: FeatureId) -> Vec<FeatureId>;

    /// Get all features that depend on this one
    pub fn dependents(&self, feature: FeatureId) -> Vec<FeatureId>;

    /// Mark feature and all dependents as dirty
    pub fn mark_dirty(&mut self, feature: FeatureId);

    /// Get recomputation order (topological sort)
    pub fn recompute_order(&self, dirty_features: &[FeatureId]) -> Vec<FeatureId>;
}
```

## Document API (Generic)

```rust
impl Document {
    /// Create a new document
    pub fn new(name: impl Into<String>) -> Self;

    /// Add a feature to the tree (generic, works with any WorkbenchFeature)
    pub fn add_feature<F: WorkbenchFeature>(
        &mut self,
        feature: F,
        name: String,
    ) -> DocumentResult<FeatureId>;

    /// Get feature data (returns JSON, workbench must deserialize)
    pub fn get_feature_data(&self, id: FeatureId) -> Option<&serde_json::Value>;

    /// Get feature metadata (id, name, dirty, etc.)
    pub fn get_feature_meta(&self, id: FeatureId) -> Option<&FeatureNode>;

    /// Update feature data (workbench provides serialized JSON)
    pub fn update_feature_data(
        &mut self,
        id: FeatureId,
        data: serde_json::Value,
    ) -> DocumentResult<()>;

    /// Mark feature dirty (triggers recomputation)
    pub fn mark_feature_dirty(&mut self, feature_id: FeatureId);

    /// Get all dirty features
    pub fn dirty_features(&self) -> Vec<FeatureId>;

    /// Get recomputation order for dirty features
    pub fn recompute_order(&self) -> Vec<FeatureId>;

    /// Get workbench storage
    pub fn get_workbench_storage(&self, wb_id: &WorkbenchId) -> Option<&WorkbenchStorage>;

    /// Get mutable workbench storage
    pub fn get_workbench_storage_mut(
        &mut self,
        wb_id: &WorkbenchId,
    ) -> Option<&mut WorkbenchStorage>;

    /// Set workbench storage
    pub fn set_workbench_storage(
        &mut self,
        wb_id: WorkbenchId,
        data: serde_json::Value,
    );
}
```

## Workbench Helper Methods

Workbenches provide convenience methods that wrap the generic API:

```rust
// In wb_sketch crate
impl Document {
    /// Add a sketch feature (convenience method)
    pub fn add_sketch_feature(
        &mut self,
        sketch: Sketch,
        name: String,
    ) -> DocumentResult<FeatureId> {
        let feature = SketchFeature { sketch, plane: SketchPlane::default() };
        self.add_feature(feature, name)
    }

    /// Get a sketch feature (convenience method)
    pub fn get_sketch_feature(&self, id: FeatureId) -> Option<SketchFeature> {
        self.get_feature_data(id)
            .and_then(|data| SketchFeature::from_json(data).ok())
    }
}
```

## Document File Format

The document is stored as a **`.prtcad` file**, which is a tar archive (optionally compressed with gzip or zstd) containing:

```
document.prtcad/
├── document.json          # Main document data (features, metadata, etc.)
├── assets/                # Referenced external files
│   ├── imported_base.step # Imported STEP file (if any)
│   ├── imported_mesh.stl  # Imported STL file (if any)
│   └── ...
└── cache/                 # Cached computed data (optional)
    ├── body_001.mesh      # Cached tessellation
    └── ...
```

### Document Structure

The `document.json` file contains:

```json
{
  "metadata": {
    "id": "...",
    "name": "My Project",
    "revision": 42,
    "dirty": false
  },
  "feature_tree": { ... },
  "workbench_storage": { ... },
  "assets": [
    {
      "id": "asset_001",
      "path": "assets/imported_base.step",
      "type": "step",
      "imported_at": 1234567890
    }
  ],
  "bodies": { ... }
}
```

### Asset References

When importing external files (STEP, STL, etc.), they are:

1. Copied into the `assets/` directory within the `.prtcad` archive
2. Referenced in the document JSON with metadata
3. Available for workbenches to reference

```rust
pub struct AssetReference {
    pub id: Uuid,
    pub path: String, // Path within the .prtcad archive
    pub asset_type: AssetType,
    pub imported_at: i64,
    pub metadata: serde_json::Value, // Additional metadata
}

pub enum AssetType {
    Step,
    Stl,
    Iges,
    Obj,
    // Future formats...
}
```

### Implementation

The document save/load API handles the tar container:

```rust
impl Document {
    /// Save document to a .prtcad file (tar archive, optionally compressed)
    /// Compression: None, Gzip, or Zstd
    pub fn save_to_file(&self, path: &Path, compression: Compression) -> DocumentResult<()>;

    /// Load document from a .prtcad file (auto-detects compression)
    pub fn load_from_file(path: &Path) -> DocumentResult<Self>;

    /// Add an external file as an asset (copies into archive)
    pub fn add_asset(&mut self, source_path: &Path, asset_type: AssetType) -> DocumentResult<Uuid>;

    /// Get asset path within the archive
    pub fn get_asset_path(&self, asset_id: Uuid) -> Option<&str>;
}

pub enum Compression {
    None,      // Plain tar
    Gzip,      // .tar.gz or .prtcad.gz
    Zstd,      // .tar.zst or .prtcad.zst
}
```

### File Extensions

- `.prtcad` - Plain tar archive (uncompressed)
- `.prtcad.gz` - Tar archive compressed with gzip
- `.prtcad.zst` - Tar archive compressed with zstd (recommended for better compression)

## Implementation Plan

1. **Phase 1: Generic Core Structure**

   - Define `FeatureId`, `BodyId` types
   - Implement generic `FeatureTree` with DAG operations
   - Create `FeatureNode` (type-erased feature storage)
   - Update `Document` with generic feature tree
   - Add `WorkbenchStorage` for workbench-specific data

2. **Phase 2: WorkbenchFeature Trait**

   - Define `WorkbenchFeature` trait
   - Add serialization/deserialization helpers
   - Implement generic `add_feature()` method

3. **Phase 3: Sketch Workbench Integration**

   - Implement `SketchFeature` with `WorkbenchFeature` trait
   - Add convenience methods in `wb_sketch` crate
   - Migrate sketch storage from workbench state to Document

4. **Phase 4: Dependency Tracking**

   - Implement dependency graph (already generic)
   - Add dirty flag propagation
   - Implement topological sort for recomputation

5. **Phase 5: Part Design Features**

   - Implement `PadFeature`, `PocketFeature`, etc. with `WorkbenchFeature`
   - Add convenience methods in `wb_part` crate
   - Link features to bodies

6. **Phase 6: Bodies**
   - Add Body storage
   - Link features to bodies
   - Support kernel handle storage
