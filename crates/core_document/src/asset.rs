//! Asset management for external files referenced in documents.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Reference to an external file stored in the document archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetReference {
    pub id: Uuid,
    /// Path within the .prtcad archive (e.g., "assets/imported_base.step").
    pub path: String,
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
    Iges,
    Obj,
    /// 3MF package
    ThreeMf,
    Ply,
    /// glTF, as JSON (`.gltf`) or binary (`.glb`)
    Gltf,
    Vrml,
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
            AssetType::ThreeMf => "3mf",
            AssetType::Ply => "ply",
            AssetType::Gltf => "gltf",
            AssetType::Vrml => "wrl",
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
            "3mf" => AssetType::ThreeMf,
            "ply" => AssetType::Ply,
            "gltf" | "glb" => AssetType::Gltf,
            "wrl" | "vrml" => AssetType::Vrml,
            _ => AssetType::Other,
        }
    }

    /// A format of triangles only: its bodies draw and pick, and take
    /// features once converted to a solid.
    pub fn is_mesh(&self) -> bool {
        matches!(
            self,
            AssetType::Stl
                | AssetType::Obj
                | AssetType::ThreeMf
                | AssetType::Ply
                | AssetType::Gltf
                | AssetType::Vrml
        )
    }
}

#[cfg(test)]
mod tests {
    use super::AssetType;

    #[test]
    fn every_mesh_format_the_importer_reads_is_a_mesh() {
        for ext in [
            "stl", "obj", "3mf", "ply", "gltf", "glb", "wrl", "vrml", "PLY",
        ] {
            assert!(AssetType::from_extension(ext).is_mesh(), "{ext}");
        }
        for ext in ["step", "stp", "iges", "igs", "dxf"] {
            assert!(!AssetType::from_extension(ext).is_mesh(), "{ext}");
        }
    }
}
