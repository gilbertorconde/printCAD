//! What an edit touches: the things of the document whose state an op
//! reads or writes. Two edits that touch nothing in common give the same
//! document in either order, so one of them can be taken back by its
//! inverse while the other stays. Where an op's reach is wide or hard to
//! name (an import, an array of copies) it touches everything.

use std::collections::HashSet;

use crate::op::DocumentOp;
use crate::{ArrayId, BodyId, ComponentId, FeatureId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Touch {
    /// One of the document's own settings, by name.
    Document(&'static str),
    Body(BodyId),
    Feature(FeatureId),
    Array(ArrayId),
    Component(ComponentId),
    ImportedObject(uuid::Uuid),
}

/// The things a set of edits touches.
#[derive(Debug, Default)]
pub(super) struct Touches {
    /// Everything: an edit whose reach is not named.
    all: bool,
    set: HashSet<Touch>,
}

impl Touches {
    /// Add what `op` touches.
    pub(super) fn add(&mut self, op: &DocumentOp) {
        use DocumentOp as Op;
        match op {
            Op::SetDocumentName { .. } => self.doc("name"),
            Op::SetDisplayUnit { .. } => self.doc("display unit"),
            Op::SetMeasureUnit { .. } => self.doc("measure unit"),
            Op::SetAgentRules { .. } => self.doc("agent rules"),
            Op::SetSavedSections { .. } => self.doc("sections"),
            Op::CreateLinkedCopy { id, source, .. } => {
                self.body(*id);
                self.body(*source);
            }
            Op::CreateBody { id, .. }
            | Op::CreateMeshBody { id, .. }
            | Op::CreateLinkedBody { id, .. }
            | Op::SetBodyLink { id, .. }
            | Op::SetBodyBase { id, .. }
            | Op::SetMeshStart { id, .. }
            | Op::SetBodyShading { id, .. }
            | Op::SetBodyFrozen { id, .. }
            | Op::SetBodyScan { id, .. }
            | Op::SetBodySelectable { id, .. }
            | Op::SetBodyMaterial { id, .. }
            | Op::SetBodyPrintRest { id, .. }
            | Op::SetBodyTextures { id, .. }
            | Op::SetFaceColor { id, .. }
            | Op::RenameBody { id, .. }
            | Op::SetBodyDisplay { id, .. }
            | Op::RequestBodyRepair { id }
            | Op::ReplaceBodyShape { id, .. }
            | Op::SetBodyVisible { id, .. }
            | Op::SetBodyPlacement { id, .. }
            | Op::RequestMeshSolid { id }
            | Op::RequestBodyRefine { id }
            | Op::RemoveBody { id } => self.body(*id),
            // The next note's number is the document's.
            Op::SetBodyNotes { id, .. } => {
                self.body(*id);
                self.doc("note numbers");
            }
            Op::SetBodyComponent { id, component } => {
                self.body(*id);
                if let Some(component) = component {
                    self.set.insert(Touch::Component(*component));
                }
            }
            Op::SetBodyStandIn { id, feature, .. } => {
                self.body(*id);
                self.feature(*feature);
            }
            Op::SetBodyTip { id, tip } => {
                self.body(*id);
                if let Some(tip) = tip {
                    self.feature(*tip);
                }
            }
            Op::SetComponent { id, .. } => {
                self.set.insert(Touch::Component(*id));
            }
            Op::AddFeature {
                id,
                body,
                deps,
                dependents,
                ..
            } => {
                self.feature(*id);
                if let Some(body) = body {
                    self.body(*body);
                }
                deps.iter().for_each(|dep| self.feature(*dep));
                dependents.iter().for_each(|(dep, _)| self.feature(*dep));
            }
            Op::UpdateFeatureData { id, .. }
            | Op::PatchFeatureData { id, .. }
            | Op::RenameFeature { id, .. }
            | Op::SetFeatureOrigin { id, .. }
            | Op::SetFeatureVisible { id, .. }
            | Op::SetFeatureSuppressed { id, .. }
            | Op::RemoveFeature { id }
            | Op::SetFeatureFormula { id, .. } => self.feature(*id),
            Op::SetFeatureDependencies { id, deps } => {
                self.feature(*id);
                deps.iter().for_each(|dep| self.feature(*dep));
            }
            Op::SwapFeatureSeq { a, b } => {
                self.feature(*a);
                self.feature(*b);
            }
            Op::SetFeatureBody { id, body, .. } => {
                self.feature(*id);
                if let Some(body) = body {
                    self.body(*body);
                }
            }
            Op::SetImportedObjectVisibility { id, .. } => {
                self.set.insert(Touch::ImportedObject(*id));
            }
            Op::SetCopyArray { id, .. } => {
                self.set.insert(Touch::Array(*id));
                self.all = true;
            }
            Op::AddAsset { .. }
            | Op::ImportModel { .. }
            | Op::AppendImportedObjectGraph { .. }
            | Op::ClearImportedObjectGraph => self.all = true,
        }
    }

    /// Add what `other` touches.
    pub(super) fn absorb(&mut self, other: Touches) {
        self.all |= other.all;
        self.set.extend(other.set);
    }

    /// Everything: what a step of history moved by undo or redo touched,
    /// not looked into.
    pub(super) fn add_all(&mut self) {
        self.all = true;
    }

    /// Whether these and `other` touch something in common.
    pub(super) fn meets(&self, other: &Touches) -> bool {
        let anything = |t: &Touches| t.all || !t.set.is_empty();
        if self.all && anything(other) || other.all && anything(self) {
            return true;
        }
        let (small, large) = match self.set.len() <= other.set.len() {
            true => (&self.set, &other.set),
            false => (&other.set, &self.set),
        };
        small.iter().any(|touch| large.contains(touch))
    }

    fn doc(&mut self, setting: &'static str) {
        self.set.insert(Touch::Document(setting));
    }

    fn body(&mut self, id: BodyId) {
        self.set.insert(Touch::Body(id));
    }

    fn feature(&mut self, id: FeatureId) {
        self.set.insert(Touch::Feature(id));
    }
}

/// What the pair touches: the op and its inverse both, since an inverse
/// can name more than its op does (a removed feature's inverse lists the
/// features that read it).
pub(super) fn of_pair(op: &DocumentOp, inverse: &DocumentOp) -> Touches {
    let mut touches = Touches::default();
    touches.add(op);
    touches.add(inverse);
    touches
}
