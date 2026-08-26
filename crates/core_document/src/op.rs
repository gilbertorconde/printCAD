//! The document's replicated operations.
//!
//! Every user edit to a [`Document`](crate::Document) is captured as one
//! [`DocumentOp`] — a **resolved effect, not an intent**: ids, timestamps and
//! sequence numbers are decided at capture time and carried in the op, so
//! applying the same op to the same state always produces the same state, on
//! this machine or a peer's. The public mutators on `Document` all follow the
//! same shape: validate and resolve, build the op, run it through
//! [`Document::apply_op`], record it. Replay therefore exercises the exact
//! code the live edit did.
//!
//! Derived state is **never** an op: dirty flags, recompute errors and the
//! imported-geometry sidecars are per-replica consequences of applying ops
//! (a peer that applies `UpdateFeatureData` marks the feature dirty itself
//! and re-derives). The replicated projection — what must converge — is the
//! serialized document minus those fields; see
//! [`Document::replicated_projection`](crate::Document::replicated_projection).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::asset::AssetReference;
use crate::feature::{BodyId, FeatureId};
use crate::units::Unit;
use crate::workbench::WorkbenchId;
use crate::ImportedObjectNode;

/// Version of the op vocabulary. Lives on the envelope (the op log / wire
/// protocol), not on each variant; additive evolution uses serde defaults.
pub const OP_PROTOCOL_VERSION: u32 = 1;

/// A blob riding inside an op (asset bytes, import sources).
///
/// Today this serializes the bytes inline; the future wire split (ops carry
/// a content hash, bytes travel separately) changes this type, not the shape
/// of any op that uses it.
#[derive(Debug, Clone)]
pub struct BlobPayload(pub std::sync::Arc<Vec<u8>>);

// Base64 on the wire and in the op log: a Vec<u8> would serialize as a JSON
// array of numbers — a 27 MB STEP becoming a ~100 MB digit list. Base64 is
// 4/3 the raw size and one string token.
impl Serialize for BlobPayload {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use base64::Engine as _;
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&*self.0))
    }
}

impl<'de> Deserialize<'de> for BlobPayload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use base64::Engine as _;
        let text = String::deserialize(deserializer)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map_err(serde::de::Error::custom)?;
        Ok(Self(std::sync::Arc::new(bytes)))
    }
}

impl BlobPayload {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(std::sync::Arc::new(bytes))
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

/// A body created by [`DocumentOp::ImportModel`], with its identity resolved
/// at capture so replay reproduces it exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedBodyInit {
    pub id: BodyId,
    pub name: String,
    pub created_at: i64,
}

/// One resolved user edit. See the module docs for the rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DocumentOp {
    SetDocumentName {
        name: String,
    },
    SetDisplayUnit {
        unit: Unit,
    },
    CreateBody {
        id: BodyId,
        name: String,
        created_at: i64,
    },
    RenameBody {
        id: BodyId,
        name: String,
    },
    SetBodyTip {
        id: BodyId,
        tip: Option<FeatureId>,
    },
    AddFeature {
        id: FeatureId,
        workbench_id: WorkbenchId,
        name: String,
        body: Option<BodyId>,
        deps: Vec<FeatureId>,
        data: serde_json::Value,
        seq: u64,
        created_at: i64,
    },
    /// Whole-payload feature write (sketch edits, panel editors). Consecutive
    /// updates to the same feature coalesce in the outbox — nothing observes
    /// the intermediate states, so the final payload is the op.
    UpdateFeatureData {
        id: FeatureId,
        data: serde_json::Value,
    },
    RenameFeature {
        id: FeatureId,
        name: String,
    },
    SetFeatureVisible {
        id: FeatureId,
        visible: bool,
    },
    SetFeatureSuppressed {
        id: FeatureId,
        suppressed: bool,
    },
    SetFeatureDependencies {
        id: FeatureId,
        deps: Vec<FeatureId>,
    },
    /// The resolved form of "move in history": the guard (peer ordering,
    /// dependency direction) runs at capture; the op is a pure seq swap.
    SwapFeatureSeq {
        a: FeatureId,
        b: FeatureId,
    },
    RemoveFeature {
        id: FeatureId,
    },
    AddAsset {
        asset: AssetReference,
        /// Empty when the asset was registered without loaded bytes.
        bytes: Option<BlobPayload>,
    },
    /// One STEP import, atomic: the asset, the bodies it created, and the
    /// object hierarchy. Geometry (meshes, B-rep snapshots) is *derived* —
    /// a replica re-derives it from the asset bytes and `detail`, which is
    /// deterministic at any thread count (see CLAUDE.md).
    ImportModel {
        asset: AssetReference,
        bytes: BlobPayload,
        detail: kernel_api::TessellationSettings,
        bodies: Vec<ImportedBodyInit>,
        roots: Vec<Uuid>,
        /// Nodes as a vec (not a map) so serialization order is stable.
        nodes: Vec<ImportedObjectNode>,
        display_unit: Option<Unit>,
    },
    /// Raw hierarchy write, also the tail step of applying `ImportModel`.
    AppendImportedObjectGraph {
        roots: Vec<Uuid>,
        nodes: Vec<ImportedObjectNode>,
    },
    SetImportedObjectVisibility {
        id: Uuid,
        visible: bool,
    },
    ClearImportedObjectGraph,
}

/// The document's outbox of captured-but-undrained ops.
///
/// `#[serde(skip)]` on the document field keeps it out of persistence, and
/// the manual [`Clone`] **returns an empty buffer**: undo baselines and save
/// snapshots are copies of *state*, not of the outbox — restoring an old
/// snapshot must not resurrect ops that were already drained to the server.
#[derive(Debug, Default)]
pub struct OpBuffer(Vec<DocumentOp>);

impl Clone for OpBuffer {
    fn clone(&self) -> Self {
        Self(Vec::new())
    }
}

impl OpBuffer {
    /// Append an op, coalescing consecutive whole-payload writes to the same
    /// feature: a drag's per-frame updates collapse to the latest payload.
    pub fn record(&mut self, op: DocumentOp) {
        if let DocumentOp::UpdateFeatureData { id, .. } = &op {
            if let Some(DocumentOp::UpdateFeatureData { id: tail_id, .. }) = self.0.last() {
                if tail_id == id {
                    *self.0.last_mut().expect("tail exists") = op;
                    return;
                }
            }
        }
        self.0.push(op);
    }

    /// Drain everything recorded since the last take.
    pub fn take(&mut self) -> Vec<DocumentOp> {
        std::mem::take(&mut self.0)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(id: FeatureId, n: u64) -> DocumentOp {
        DocumentOp::UpdateFeatureData {
            id,
            data: serde_json::json!({ "n": n }),
        }
    }

    #[test]
    fn consecutive_updates_to_one_feature_coalesce_to_the_last() {
        let id = FeatureId::new();
        let other = FeatureId::new();
        let mut buffer = OpBuffer::default();
        buffer.record(update(id, 1));
        buffer.record(update(id, 2));
        buffer.record(update(other, 3));
        buffer.record(update(id, 4));

        let ops = buffer.take();
        assert_eq!(ops.len(), 3, "the first two collapse; the rest interleave");
        match &ops[0] {
            DocumentOp::UpdateFeatureData { data, .. } => assert_eq!(data["n"], 2),
            other => panic!("expected update, got {other:?}"),
        }
    }

    #[test]
    fn cloning_the_buffer_yields_an_empty_one() {
        let mut buffer = OpBuffer::default();
        buffer.record(update(FeatureId::new(), 1));
        let copy = buffer.clone();
        assert!(copy.is_empty(), "snapshots carry state, never the outbox");
        assert_eq!(buffer.len(), 1, "the original keeps its pending ops");
    }
}
