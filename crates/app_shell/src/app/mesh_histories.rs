//! Mesh bodies' histories built away from the window: each body's plan
//! (the benches' `mesh_plans`) applied to its mesh start on a thread of its
//! own, the result the body's mesh (derived; the start and the features
//! are what is saved). A body whose plan is empty draws its start again.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use core_document::{BodyId, ImportedGeometry, MeshPlan};

/// What a build answers.
type Answer = Result<kernel_api::MeshChainAnswer, String>;

/// What a build that ended without an answer answers.
const ENDED: &str = "The mesh history's build stopped without an answer";

/// A build under way: what it is built from, and where its answer comes.
struct Building {
    key: u64,
    plan: MeshPlan,
    answer: Option<Receiver<Answer>>,
}

/// The tab's mesh histories: built, being built.
#[derive(Default)]
pub(crate) struct MeshHistories {
    built: HashMap<BodyId, u64>,
    making: HashMap<BodyId, Building>,
}

impl MeshHistories {
    /// Whether a history is being built: frames keep coming until it lands.
    pub(crate) fn pending(&self) -> bool {
        !self.making.is_empty()
    }

    /// The builds that have answered, taken out with their key and plan;
    /// one that ended without an answer (its thread panicked) is said in
    /// the log and answers [`ENDED`], so it never keeps frames coming and
    /// its first step carries the failure.
    fn take_finished(&mut self) -> Vec<(BodyId, u64, MeshPlan, Answer)> {
        let finished: Vec<(BodyId, Answer)> = self
            .making
            .iter_mut()
            .filter_map(|(body, building)| {
                let answer = crate::platform::take_answer(&mut building.answer)?;
                Some((
                    *body,
                    answer.unwrap_or_else(|_| {
                        crate::log_panel::warn(format!("{ENDED}."));
                        Err(ENDED.to_string())
                    }),
                ))
            })
            .collect();
        finished
            .into_iter()
            .filter_map(|(body, answer)| {
                let Building { key, plan, .. } = self.making.remove(&body)?;
                Some((body, key, plan, answer))
            })
            .collect()
    }
}

/// What a history is built from: the start mesh and the steps.
fn key_of(start: &ImportedGeometry, plan: &MeshPlan) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    start.revision.hash(&mut h);
    Arc::as_ptr(&start.mesh).hash(&mut h);
    serde_json::to_string(&plan.ops)
        .unwrap_or_default()
        .hash(&mut h);
    plan.features.hash(&mut h);
    h.finish()
}

impl crate::PrintCadApp {
    /// Take in the histories built, and start a build for each body whose
    /// plan changed.
    pub(crate) fn drive_mesh_histories(&mut self) {
        let landed = self.session.jobs.mesh_histories.take_finished();
        for (body, key, plan, answer) in landed {
            let current = self.registry.mesh_plans(&self.session.document);
            if !current.contains(&plan)
                || self
                    .session
                    .document
                    .mesh_start(body)
                    .is_none_or(|start| key_of(start, &plan) != key)
            {
                continue;
            }
            self.session.jobs.mesh_histories.built.insert(body, key);
            plan.land(&mut self.session.document, answer);
        }

        let plans = self.registry.mesh_build_plans(&self.session.document);
        for plan in plans {
            let Some(start) = self.session.document.mesh_start(plan.body) else {
                continue;
            };
            let key = key_of(start, &plan);
            let histories = &mut self.session.jobs.mesh_histories;
            if histories.built.get(&plan.body) == Some(&key)
                || histories.making.get(&plan.body).map(|b| b.key) == Some(key)
            {
                continue;
            }
            if let Some(answer) = plan.restored_start(&start.mesh) {
                histories.making.remove(&plan.body);
                histories.built.insert(plan.body, key);
                plan.land(&mut self.session.document, Ok(answer));
                continue;
            }
            let (tx, rx) = channel();
            let mesh = Arc::clone(&start.mesh);
            let body = plan.body;
            let ops = plan.ops.clone();
            histories.making.insert(
                body,
                Building {
                    key,
                    plan,
                    answer: Some(rx),
                },
            );
            let spawned = crate::platform::spawn("printcad-mesh-history", move || {
                use kernel_api::KernelQueries;
                let answer = kernel_ogeom::QUERIES
                    .build_mesh_chain(&mesh, &ops)
                    .map_err(|e| e.to_string());
                let _ = tx.send(answer);
            });
            if let Err(err) = spawned {
                crate::log_panel::warn(format!("A mesh history could not start: {err}"));
                self.session.jobs.mesh_histories.making.remove(&body);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A build whose thread ends without answering is taken out with a
    /// failure, so nothing is left pending.
    #[test]
    fn a_build_that_dies_is_taken_out_with_a_failure() {
        let mut histories = MeshHistories::default();
        let body = BodyId(uuid::Uuid::nil());
        let (tx, rx) = channel();
        let plan = MeshPlan {
            body,
            ops: Vec::new(),
            features: Vec::new(),
        };
        let answer = Some(rx);
        histories.making.insert(
            body,
            Building {
                key: 3,
                plan,
                answer,
            },
        );
        assert!(histories.take_finished().is_empty() && histories.pending());
        drop(tx);
        let finished = histories.take_finished();
        assert!(matches!(&finished[..], [(b, 3, _, Err(_))] if *b == body));
        assert!(!histories.pending());
    }
}
