//! Mesh bodies' histories built away from the window: each body's plan
//! (the benches' `mesh_plans`) applied to its mesh start on a thread of its
//! own, the result the body's mesh (derived; the start and the features
//! are what is saved). A body whose plan is empty draws its start again.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use core_document::{BodyId, ImportedGeometry, MeshPlan};

/// What one build came to.
struct Landed {
    body: BodyId,
    key: u64,
    plan: MeshPlan,
    answer: Result<kernel_api::MeshChainAnswer, String>,
}

/// The tab's mesh histories: built, being built.
#[derive(Default)]
pub(crate) struct MeshHistories {
    built: HashMap<BodyId, u64>,
    making: HashMap<BodyId, u64>,
    channel: Option<(Sender<Landed>, Receiver<Landed>)>,
}

impl MeshHistories {
    /// Whether a history is being built: frames keep coming until it lands.
    pub(crate) fn pending(&self) -> bool {
        !self.making.is_empty()
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
        let histories = &mut self.session.mesh_histories;
        let mut landed = Vec::new();
        if let Some((_, rx)) = &histories.channel {
            while let Ok(done) = rx.try_recv() {
                landed.push(done);
            }
        }
        for done in landed {
            let histories = &mut self.session.mesh_histories;
            if histories.making.get(&done.body) != Some(&done.key) {
                continue;
            }
            histories.making.remove(&done.body);
            let current = self.registry.mesh_plans(&self.session.document);
            if !current.contains(&done.plan)
                || self
                    .session
                    .document
                    .mesh_start(done.body)
                    .is_none_or(|start| key_of(start, &done.plan) != done.key)
            {
                continue;
            }
            self.session
                .mesh_histories
                .built
                .insert(done.body, done.key);
            match done.answer {
                Ok(answer) => self.session.document.store_mesh_build(done.plan, answer),
                Err(why) => {
                    if let Some(id) = done.plan.features.first() {
                        self.session.document.set_feature_error(*id, Some(why));
                    }
                }
            }
        }

        let document = &self.session.document;
        let plans = self.registry.mesh_plans(document);
        let mut planned: Vec<BodyId> = Vec::new();
        for plan in plans {
            planned.push(plan.body);
            let Some(start) = document.mesh_start(plan.body) else {
                continue;
            };
            let key = key_of(start, &plan);
            let histories = &mut self.session.mesh_histories;
            if histories.built.get(&plan.body) == Some(&key)
                || histories.making.get(&plan.body) == Some(&key)
            {
                continue;
            }
            histories.making.insert(plan.body, key);
            let tx = histories.channel.get_or_insert_with(channel).0.clone();
            let mesh = Arc::clone(&start.mesh);
            let body = plan.body;
            let spawned = crate::platform::spawn("printcad-mesh-history", move || {
                use kernel_api::KernelQueries;
                let answer = kernel_ogeom::QUERIES
                    .build_mesh_chain(&mesh, &plan.ops)
                    .map_err(|e| e.to_string());
                let _ = tx.send(Landed {
                    body,
                    key,
                    plan,
                    answer,
                });
            });
            if let Err(err) = spawned {
                crate::log_panel::warn(format!("A mesh history could not start: {err}"));
                self.session.mesh_histories.making.remove(&body);
            }
        }
        // A start no plan builds from any more is the body's mesh again.
        let unplanned: Vec<(BodyId, ImportedGeometry)> = self
            .session
            .document
            .bodies()
            .iter()
            .filter(|b| !planned.contains(&b.id))
            .filter_map(|b| Some((b.id, self.session.document.mesh_start(b.id)?.clone())))
            .collect();
        for (body, start) in unplanned {
            let plan = MeshPlan {
                body,
                ops: Vec::new(),
                features: Vec::new(),
            };
            let key = key_of(&start, &plan);
            if self.session.mesh_histories.built.get(&body) == Some(&key) {
                continue;
            }
            self.session.mesh_histories.making.remove(&body);
            self.session.mesh_histories.built.insert(body, key);
            self.session.document.store_mesh_build(
                plan,
                kernel_api::MeshChainAnswer {
                    mesh: (*start.mesh).clone(),
                    ..Default::default()
                },
            );
        }
    }
}
