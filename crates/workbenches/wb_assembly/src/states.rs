//! Saved assembly states: where every body sat, which were hidden, and
//! where each driven joint was held, kept in the document under a name to
//! return to (a print-in-place hinge folded, and open).

use core_document::{
    BodyId, BodyPlacement, Document, DocumentResult, FeatureError, FeatureId, WorkbenchFeature,
    WorkbenchId,
};
use serde::{Deserialize, Serialize};

use crate::joint::{JointFeature, JointKind};
use crate::solve::joints;

/// The feature kind saved states are stored as.
pub const STATE_KIND: &str = "wb.assembly.state";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AssemblyState {
    pub placements: Vec<(BodyId, BodyPlacement)>,
    pub hidden: Vec<BodyId>,
    /// Each driven joint and the value its drive held.
    pub drives: Vec<(FeatureId, f32)>,
}

impl WorkbenchFeature for AssemblyState {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(STATE_KIND)
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Assembly state"
    }
}

/// The assembly as it stands.
pub fn capture(document: &Document) -> AssemblyState {
    AssemblyState {
        placements: document
            .bodies()
            .iter()
            .map(|b| (b.id, b.placement))
            .collect(),
        hidden: document
            .bodies()
            .iter()
            .filter(|b| b.hidden)
            .map(|b| b.id)
            .collect(),
        drives: joints(document)
            .into_iter()
            .filter_map(|j| match j.feature.kind {
                JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } => {
                    Some((j.id, drive.to?))
                }
                _ => None,
            })
            .collect(),
    }
}

/// Save the assembly as it stands under `name`; the state's id.
pub fn save(document: &mut Document, name: String) -> DocumentResult<FeatureId> {
    let state = capture(document);
    let id = document.add_feature_in_body(state, name, None)?;
    document.clear_feature_dirty(id);
    Ok(id)
}

/// Put the assembly back as `state` has it: drives, placements, and
/// which bodies show. Bodies made since keep where they are.
pub fn restore(document: &mut Document, state: &AssemblyState) {
    for (joint, value) in &state.drives {
        let Some(mut feature) = document
            .get_feature_data(*joint)
            .and_then(|d| JointFeature::from_json(d).ok())
        else {
            continue;
        };
        if let JointKind::Hinge { drive, .. } | JointKind::Slider { drive, .. } = &mut feature.kind
            && drive.to.is_some()
        {
            drive.to = Some(*value);
            if document
                .update_feature_data(*joint, feature.to_json())
                .is_ok()
            {
                document.clear_feature_dirty(*joint);
            }
        }
    }
    for (body, placement) in &state.placements {
        if document.body(*body).is_some() {
            document.set_body_placement(*body, *placement);
        }
    }
    let bodies: Vec<BodyId> = document.bodies().iter().map(|b| b.id).collect();
    let known: Vec<BodyId> = state.placements.iter().map(|(b, _)| *b).collect();
    for body in bodies.into_iter().filter(|b| known.contains(b)) {
        document.set_body_visible(body, !state.hidden.contains(&body));
    }
}
