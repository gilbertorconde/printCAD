//! The document's replicated operations.
//!
//! Every user edit to a [`Document`](crate::Document) is captured as one
//! [`DocumentOp`]: a **resolved effect, not an intent**. Ids, timestamps and
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
//! and re-derives). The replicated projection, what must converge, is the
//! serialized document minus those fields; see
//! [`Document::replicated_projection`](crate::Document::replicated_projection).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ImportedObjectNode;
use crate::asset::AssetReference;
use crate::feature::{BodyId, FeatureId};
use crate::units::Unit;
use crate::workbench::WorkbenchId;

/// Version of the op vocabulary. Lives on the envelope (the op log / wire
/// protocol), not on each variant; additive evolution uses serde defaults.
pub const OP_PROTOCOL_VERSION: u32 = 1;

/// A blob riding inside an op (asset bytes, import sources).
///
/// The bytes serialize inline. Carrying a content hash with the bytes
/// travelling separately would change this type, not the shape of any op
/// that uses it.
#[derive(Debug, Clone)]
pub struct BlobPayload(pub std::sync::Arc<Vec<u8>>);

// Base64 on the wire and in the op log: a Vec<u8> would serialize as a JSON
// array of numbers, a 27 MB STEP becoming a ~100 MB digit list. Base64 is
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
    /// The unit the measure tool reads in; `None` is the display unit.
    SetMeasureUnit {
        unit: Option<Unit>,
    },
    /// What an AI agent working on this document is told to follow.
    SetAgentRules {
        rules: String,
    },
    /// The document's named sections, the whole list.
    SetSavedSections {
        sections: Vec<crate::sections::SavedSection>,
    },
    CreateBody {
        id: BodyId,
        name: String,
        created_at: i64,
    },
    /// A body that takes another's shape: a linked copy of `source`.
    CreateLinkedCopy {
        id: BodyId,
        name: String,
        created_at: i64,
        source: BodyId,
        /// The plane, in the source's frame, a mirrored copy is mirrored
        /// across.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mirror: Option<crate::MirrorPlane>,
    },
    /// A mesh body holding a mesh the document made itself (a piece split
    /// off another mesh body): `mesh` a `TriMesh` packed as a `.prtcad`
    /// mesh entry is, in the body's own frame, read under `asset`, which
    /// carries no bytes.
    CreateMeshBody {
        id: BodyId,
        name: String,
        created_at: i64,
        asset: AssetReference,
        mesh: BlobPayload,
    },
    /// A part linked from another printCAD file.
    CreateLinkedBody {
        id: BodyId,
        name: String,
        created_at: i64,
        link: crate::FileLink,
    },
    /// A linked part read again from its file, now at `link.stamp`.
    SetBodyLink {
        id: BodyId,
        link: crate::FileLink,
    },
    /// An array of linked copies made, changed, or (for `None`) taken
    /// away, whole: the copies it lists are made where missing, the ones it
    /// no longer lists go, and every one is placed by the pattern.
    SetCopyArray {
        id: crate::ArrayId,
        array: Option<crate::CopyArray>,
        /// Copies to make as they were, their own settings with them,
        /// rather than afresh: what undoing a smaller array or its removal
        /// brings back.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        restored: Vec<crate::Body>,
    },
    /// A component made, changed, or (for `None`) taken away.
    SetComponent {
        id: crate::ComponentId,
        component: Option<crate::Component>,
    },
    /// A body's solid kept as the base its history starts from (`on`), or
    /// the base put back as its shape.
    SetBodyBase {
        id: BodyId,
        on: bool,
    },
    /// A mesh body's mesh kept as the start its mesh history builds from
    /// (`on`), or the start put back as its mesh.
    SetMeshStart {
        id: BodyId,
        on: bool,
    },
    /// How a mesh body's triangles are shaded.
    SetBodyShading {
        id: BodyId,
        shading: crate::Shading,
    },
    /// A body kept as it stands, or let rebuild.
    SetBodyFrozen {
        id: BodyId,
        frozen: bool,
    },
    /// A mesh body marked as a scan, or as an ordinary mesh again.
    SetBodyScan {
        id: BodyId,
        scan: Option<crate::ScanInfo>,
    },
    SetBodySelectable {
        id: BodyId,
        selectable: bool,
    },
    SetBodyMaterial {
        id: BodyId,
        material: Option<crate::Material>,
    },
    /// How a body lies in the print layout, `None` for the face the
    /// layout finds.
    SetBodyPrintRest {
        id: BodyId,
        rest: Option<crate::PrintRest>,
    },
    /// The textures pressed into a body's faces, all of them.
    SetBodyTextures {
        id: BodyId,
        textures: Vec<crate::FaceTexture>,
    },
    /// The notes on a body, all of them, and the highest number a note of
    /// the document was given, which the next note follows.
    SetBodyNotes {
        id: BodyId,
        notes: Vec<crate::Note>,
        #[serde(default)]
        numbered: u32,
    },
    /// A face's own colour, `None` to take it away. Logs that wrote the
    /// face with a colour of its own still read: that field is ignored.
    SetFaceColor {
        id: BodyId,
        face: crate::FaceKey,
        color: Option<[f32; 3]>,
    },
    /// A body put in a component, or at the top for `None`.
    SetBodyComponent {
        id: BodyId,
        component: Option<crate::ComponentId>,
    },
    RenameBody {
        id: BodyId,
        name: String,
    },
    SetBodyDisplay {
        id: BodyId,
        display: Option<crate::BodyDisplay>,
    },
    /// Ask for the kernel's repair on an imported body's shape. The
    /// repaired geometry is derived, as an import's is.
    RequestBodyRepair {
        id: BodyId,
    },
    /// Read a body's shape from another file (`asset`, carried whole as
    /// an import's is): its first solid becomes the body's base, or its
    /// shape when it has no history. Derived, as an import's geometry is.
    ReplaceBodyShape {
        id: BodyId,
        asset: AssetReference,
        bytes: BlobPayload,
    },
    /// Keep the shape `asset` holds (carried whole, as an import's is) to
    /// stand in for the body's solid while its history does not build up to
    /// `feature`. Derived, as an import's geometry is.
    SetBodyStandIn {
        id: BodyId,
        feature: FeatureId,
        asset: AssetReference,
        bytes: BlobPayload,
    },
    /// Show or hide a body in the scene.
    SetBodyVisible {
        id: BodyId,
        visible: bool,
    },
    /// Move a body: where its own geometry sits in the document.
    SetBodyPlacement {
        id: BodyId,
        placement: crate::BodyPlacement,
    },
    /// Ask for a mesh body to become a B-rep solid. The solid is derived,
    /// as an import's geometry is.
    RequestMeshSolid {
        id: BodyId,
    },
    /// Ask for a converted solid's facets to be rebuilt on the surfaces
    /// they approximate. The refined solid is derived, as an import's
    /// geometry is.
    RequestBodyRefine {
        id: BodyId,
    },
    /// Delete a body: the inverse of `CreateBody`, and what the tree's
    /// Delete does. Any features still attached go with it, along with the
    /// body's geometry.
    RemoveBody {
        id: BodyId,
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
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        formulas: std::collections::BTreeMap<String, String>,
        /// The workbench package and version that made it, when a loaded
        /// package did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        made_by: Option<String>,
        /// Where that package is published, when it was installed from a
        /// repository.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package_source: Option<String>,
        /// The features that read it, each with the place it takes among
        /// what that feature reads: what undoing its removal puts back. A
        /// listed feature that is not there is passed over.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        dependents: Vec<(FeatureId, usize)>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        hidden: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        suppressed: bool,
    },
    /// Whole-payload feature write (sketch edits, panel editors). Consecutive
    /// updates to the same feature coalesce in the outbox: nothing observes
    /// the intermediate states, so the final payload is the op. The payload
    /// is shared: the journal's copy of the op and the outbox's are one
    /// value, written on the wire as the value itself.
    UpdateFeatureData {
        id: FeatureId,
        data: std::sync::Arc<serde_json::Value>,
    },
    /// Writes into a feature's data where it stands, the rest of it left
    /// as it is: an edit of a large payload (a sketch) that changed a few
    /// of its parts. Its inverse is a patch too, worked out from the data
    /// it is applied to. A patch that does not fit the data it meets (an
    /// item it names is gone) changes nothing. Consecutive patches of one
    /// feature join in the outbox and the journal as updates do.
    PatchFeatureData {
        id: FeatureId,
        patch: crate::patch::DataPatch,
    },
    RenameFeature {
        id: FeatureId,
        name: String,
    },
    /// The package (and version) that last wrote the feature, and where
    /// that package is published.
    SetFeatureOrigin {
        id: FeatureId,
        made_by: Option<String>,
        package_source: Option<String>,
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
    /// A feature moved into another body's history, at `seq` in it.
    SetFeatureBody {
        id: FeatureId,
        body: Option<BodyId>,
        seq: u64,
    },
    AddAsset {
        asset: AssetReference,
        /// Empty when the asset was registered without loaded bytes.
        bytes: Option<BlobPayload>,
    },
    /// One file import, atomic: the asset, the bodies it created, and the
    /// object hierarchy. Geometry (meshes, B-rep snapshots) is *derived*:
    /// a replica re-derives it from the asset bytes and `detail`, which is
    /// deterministic at any thread count (see the `printcad-kernel` skill).
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
    /// Set the formula behind a feature's number `key`, or take it away.
    SetFeatureFormula {
        id: FeatureId,
        key: String,
        formula: Option<String>,
    },
}

/// An op this replica applied and the server has not yet placed in the
/// document's order, with the op that undoes it here (`None` when it has
/// none).
#[derive(Debug)]
pub(crate) struct Unconfirmed {
    pub(crate) op: DocumentOp,
    pub(crate) inverse: Option<DocumentOp>,
}

/// The document's outbox of captured-but-undrained ops, and the batches
/// drained to the server that it has not yet confirmed.
///
/// `#[serde(skip)]` on the document field keeps it out of persistence, and
/// the manual [`Clone`] **returns an empty buffer**: undo baselines and save
/// snapshots are copies of *state*, not of the outbox; restoring an old
/// snapshot must not resurrect ops that were already drained to the server.
///
/// Each op keeps its inverse so an edit from another client that the server
/// ordered before these can be applied beneath them
/// ([`Document::apply_server_ops`](crate::Document::apply_server_ops)).
#[derive(Debug, Default)]
pub struct OpBuffer {
    pending: Vec<Unconfirmed>,
    in_flight: std::collections::VecDeque<SentBatch>,
}

/// A batch taken for the server and not yet confirmed: its id, which the
/// server logs it under and knows it again by when it is sent once more,
/// and its ops.
#[derive(Debug)]
pub(crate) struct SentBatch {
    pub(crate) id: Uuid,
    pub(crate) ops: Vec<Unconfirmed>,
}

impl Clone for OpBuffer {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl OpBuffer {
    /// Append an op, coalescing consecutive whole-payload writes to the same
    /// feature: a drag's per-frame updates collapse to the latest payload,
    /// which keeps the first one's inverse.
    pub fn record(&mut self, op: DocumentOp, inverse: Option<DocumentOp>) {
        let (op, inverse) = match (op, self.pending.last_mut()) {
            (
                DocumentOp::PatchFeatureData { id, patch },
                Some(Unconfirmed {
                    op:
                        DocumentOp::PatchFeatureData {
                            id: tail,
                            patch: joined,
                        },
                    inverse: Some(DocumentOp::PatchFeatureData { patch: undo, .. }),
                }),
            ) if *tail == id => {
                if let Some(DocumentOp::PatchFeatureData { patch: first, .. }) = inverse {
                    joined.then(patch);
                    undo.after(first);
                    return;
                }
                (DocumentOp::PatchFeatureData { id, patch }, inverse)
            }
            (op, _) => (op, inverse),
        };
        // A colour picker drag is one look per frame; the last is the op.
        let same_target = match (&op, self.pending.last().map(|u| &u.op)) {
            (
                DocumentOp::UpdateFeatureData { id, .. },
                Some(DocumentOp::UpdateFeatureData { id: tail, .. }),
            ) => tail == id,
            (
                DocumentOp::SetBodyDisplay { id, .. },
                Some(DocumentOp::SetBodyDisplay { id: tail, .. }),
            ) => tail == id,
            _ => false,
        };
        if same_target {
            self.pending.last_mut().expect("tail exists").op = op;
            return;
        }
        self.pending.push(Unconfirmed { op, inverse });
    }

    /// Drain everything recorded since the last take, as one batch for the
    /// server; the batch waits for [`Self::confirm`].
    pub fn take(&mut self) -> Vec<DocumentOp> {
        self.take_batch().map(|(_, ops)| ops).unwrap_or_default()
    }

    /// [`Self::take`] with the id the batch is sent under; `None` when
    /// nothing was recorded.
    pub fn take_batch(&mut self) -> Option<(Uuid, Vec<DocumentOp>)> {
        if self.pending.is_empty() {
            return None;
        }
        let ops = std::mem::take(&mut self.pending);
        let id = Uuid::new_v4();
        let taken = ops.iter().map(|u| u.op.clone()).collect();
        self.in_flight.push_back(SentBatch { id, ops });
        Some((id, taken))
    }

    /// The batches taken and not yet confirmed, oldest first, as they
    /// were sent: what a new connection is sent again.
    pub fn sent(&self) -> Vec<(Uuid, Vec<DocumentOp>)> {
        self.in_flight
            .iter()
            .map(|batch| (batch.id, batch.ops.iter().map(|u| u.op.clone()).collect()))
            .collect()
    }

    /// The server placed the oldest batch taken in the document's order.
    pub fn confirm(&mut self) {
        self.in_flight.pop_front();
    }

    /// Drop the batches no server will confirm: the connection they went
    /// out on is gone.
    pub fn forget_in_flight(&mut self) {
        self.in_flight.clear();
    }

    /// Batches taken and not yet confirmed.
    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    /// Every op not yet confirmed, oldest first: the batches in flight,
    /// then the outbox. Taken out; [`Self::restore`] puts them back.
    pub(crate) fn take_unconfirmed(&mut self) -> (Vec<SentBatch>, Vec<Unconfirmed>) {
        (
            std::mem::take(&mut self.in_flight).into(),
            std::mem::take(&mut self.pending),
        )
    }

    pub(crate) fn restore(&mut self, in_flight: Vec<SentBatch>, pending: Vec<Unconfirmed>) {
        self.in_flight = in_flight.into();
        self.pending = pending;
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// (op, inverse) pairs since the last journal boundary. `None` inverse =
/// history barrier (the op cannot be inverted; undo history clears).
///
/// Same clone-empty rule as [`OpBuffer`]: snapshots carry state, never
/// journal material. Coalescing keeps the LAST op with the FIRST inverse:
/// a drag undoes to where it started, not to its second-to-last frame.
#[derive(Debug, Default)]
pub struct JournalBuffer {
    pairs: Vec<(DocumentOp, DocumentOp)>,
    barrier: bool,
    /// The pairs before this index were there when the buffer was sealed:
    /// nothing later coalesces into them, and [`take_since_seal`]
    /// (Self::take_since_seal) leaves them.
    sealed: usize,
    /// A barrier crossed since the seal.
    crossed_since_seal: bool,
}

impl Clone for JournalBuffer {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl JournalBuffer {
    pub fn record(&mut self, op: &DocumentOp, inverse: Option<DocumentOp>) {
        let Some(inverse) = inverse else {
            self.barrier = true;
            self.pairs.clear();
            self.sealed = 0;
            self.crossed_since_seal = true;
            return;
        };
        if self.pairs.len() <= self.sealed {
            self.pairs.push((op.clone(), inverse));
            return;
        }
        if let (
            DocumentOp::PatchFeatureData { id, patch },
            DocumentOp::PatchFeatureData { patch: undo, .. },
        ) = (op, &inverse)
            && let Some((
                DocumentOp::PatchFeatureData {
                    id: tail_id,
                    patch: joined,
                },
                DocumentOp::PatchFeatureData {
                    patch: joined_undo, ..
                },
            )) = self.pairs.last_mut()
            && tail_id == id
        {
            // Every write forward, the first ones back.
            joined.then(patch.clone());
            joined_undo.after(undo.clone());
            return;
        }
        if let DocumentOp::UpdateFeatureData { id, .. } = op
            && let Some((DocumentOp::UpdateFeatureData { id: tail_id, .. }, _)) = self.pairs.last()
            && tail_id == id
        {
            // Latest payload forward, oldest payload back.
            self.pairs.last_mut().expect("tail exists").0 = op.clone();
            return;
        }
        self.pairs.push((op.clone(), inverse));
    }

    /// Everything since the last take, plus whether a barrier was crossed.
    pub fn take(&mut self) -> (Vec<(DocumentOp, DocumentOp)>, bool) {
        self.unseal();
        let barrier = std::mem::take(&mut self.barrier);
        (std::mem::take(&mut self.pairs), barrier)
    }

    /// Keep what is here from what comes after: later pairs never
    /// coalesce into these, and [`take_since_seal`](Self::take_since_seal)
    /// takes only the later ones.
    pub fn seal(&mut self) {
        self.sealed = self.pairs.len();
        self.crossed_since_seal = false;
    }

    /// Lift the seal: the buffer is one run of pairs again.
    pub fn unseal(&mut self) {
        self.sealed = 0;
        self.crossed_since_seal = false;
    }

    /// The pairs since the seal, plus whether a barrier was crossed since
    /// it; the ones before it stay, sealed.
    pub fn take_since_seal(&mut self) -> (Vec<(DocumentOp, DocumentOp)>, bool) {
        let since = self.pairs.split_off(self.sealed.min(self.pairs.len()));
        (since, std::mem::take(&mut self.crossed_since_seal))
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty() && !self.barrier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(id: FeatureId, n: u64) -> DocumentOp {
        DocumentOp::UpdateFeatureData {
            id,
            data: serde_json::json!({ "n": n }).into(),
        }
    }

    #[test]
    fn consecutive_updates_to_one_feature_coalesce_to_the_last() {
        let id = FeatureId::new();
        let other = FeatureId::new();
        let mut buffer = OpBuffer::default();
        buffer.record(update(id, 1), None);
        buffer.record(update(id, 2), None);
        buffer.record(update(other, 3), None);
        buffer.record(update(id, 4), None);

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
        buffer.record(update(FeatureId::new(), 1), None);
        let copy = buffer.clone();
        assert!(copy.is_empty(), "snapshots carry state, never the outbox");
        assert_eq!(buffer.len(), 1, "the original keeps its pending ops");
    }
}
