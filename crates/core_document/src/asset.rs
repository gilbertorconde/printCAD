//! Asset management for external files referenced in documents.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Reference to an external file stored in the document archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetReference {
    /// Unique identifier for this asset.
    pub id: Uuid,
    /// Path within the .prtcad archive (e.g., "assets/imported_base.step").
    pub path: String,
    /// Type of asset.
    pub asset_type: AssetType,
    /// Timestamp when asset was imported (epoch milliseconds).
    pub imported_at: i64,
    /// Additional metadata (workbench-specific, format-specific, etc.).
    pub metadata: serde_json::Value,
}

impl AssetReference {
    pub fn new(
        path: impl Into<String>,
        asset_type: AssetType,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            path: path.into(),
            asset_type,
            imported_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64,
            metadata,
        }
    }
}

/// Type of external asset file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetType {
    /// STEP file (ISO 10303)
    Step,
    /// STL file (stereolithography)
    Stl,
    /// IGES file
    Iges,
    /// OBJ file
    Obj,
    /// Other/unknown format
    Other,
}

impl AssetType {
    /// Get file extension for this asset type.
    pub fn extension(&self) -> &'static str {
        match self {
            AssetType::Step => "step",
            AssetType::Stl => "stl",
            AssetType::Iges => "iges",
            AssetType::Obj => "obj",
            AssetType::Other => "bin",
        }
    }

    /// Detect asset type from file extension.
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "step" | "stp" => AssetType::Step,
            "stl" => AssetType::Stl,
            "iges" | "igs" => AssetType::Iges,
            "obj" => AssetType::Obj,
            _ => AssetType::Other,
        }
    }
}
