//! A rigid group: several bodies locked together as they sat when it was
//! made, in one feature rather than a fixed joint for each pair. It
//! belongs to its first body; every other member is held to that one, as
//! a fixed joint would hold it, so the group moves as one.

use core_document::{BodyId, Document, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

use crate::joint::{Anchor, JointFeature, JointKind, Rigid, relative};
use crate::solve::Joint;

/// The feature kind rigid groups are stored as.
pub const GROUP_KIND: &str = "wb.assembly.group";

/// A member of a group: where it sits in the first member's frame.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub body: BodyId,
    pub turn: [f64; 4],
    pub shift: [f64; 3],
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RigidGroup {
    /// The members, the first the one the others hold to.
    pub members: Vec<Member>,
}

impl RigidGroup {
    /// `bodies` locked where they sit now, the first the one the rest
    /// hold to.
    pub fn of(document: &Document, bodies: &[BodyId]) -> Self {
        let at = |b: BodyId| Rigid::from(document.body_placement(b));
        let Some(first) = bodies.first() else {
            return Self::default();
        };
        let base = at(*first);
        Self {
            members: bodies
                .iter()
                .map(|b| {
                    let (turn, shift) = relative(&at(*b), &base);
                    Member {
                        body: *b,
                        turn: turn.to_array(),
                        shift: shift.to_array(),
                    }
                })
                .collect(),
        }
    }

    pub fn bodies(&self) -> Vec<BodyId> {
        self.members.iter().map(|m| m.body).collect()
    }
}

impl WorkbenchFeature for RigidGroup {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(GROUP_KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        "Rigid group"
    }
}

/// Every group that is not suppressed, with its id and name, in history
/// order.
pub fn groups(document: &Document) -> Vec<(FeatureId, String, RigidGroup)> {
    let mut found: Vec<(u64, FeatureId, String, RigidGroup)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == GROUP_KIND && !n.suppressed)
        .filter_map(|(id, n)| {
            Some((
                n.seq,
                *id,
                n.name.clone(),
                RigidGroup::from_json(&n.data).ok()?,
            ))
        })
        .collect();
    found.sort_by_key(|(seq, id, ..)| (*seq, *id));
    found
        .into_iter()
        .map(|(_, id, name, group)| (id, name, group))
        .collect()
}

/// Each group's members after the first, as the solver reads them: held
/// to the first as a fixed joint holds a body, under the group's id and
/// name.
pub fn holds(document: &Document) -> Vec<Joint> {
    let origin = Anchor::Plane {
        point: [0.0; 3],
        normal: [0.0, 0.0, 1.0],
    };
    groups(document)
        .into_iter()
        .flat_map(|(id, name, group)| {
            let Some(first) = group.members.first().map(|m| m.body) else {
                return Vec::new();
            };
            group
                .members
                .iter()
                .skip(1)
                .map(|m| Joint {
                    id,
                    name: name.clone(),
                    body: m.body,
                    feature: JointFeature {
                        second: None,
                        shape: Vec::new(),
                        ends: [0.0; 2],
                        names: [0; 2],
                        kind: JointKind::Fixed {
                            turn: m.turn,
                            shift: m.shift,
                        },
                        moving: origin,
                        other_body: first,
                        fixed: origin,
                    },
                })
                .collect()
        })
        .collect()
}
