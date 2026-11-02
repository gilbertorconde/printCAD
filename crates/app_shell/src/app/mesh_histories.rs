//! Mesh bodies' histories built away from the window: each body's plan
//! (the benches' `mesh_plans`) applied to its mesh start on a thread of its
//! own, the result the body's mesh (derived; the start and the features
//! are what is saved). A body whose plan is empty draws its start again.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use core_document::{BodyId, FeatureId, ImportedGeometry};

/// What one build came to.
struct Landed {
    body: BodyId,
    key: u64,
    features: Vec<FeatureId>,
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
fn key_of(start: &ImportedGeometry, ops: &[kernel_api::MeshOp]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    start.revision.hash(&mut h);
    Arc::as_ptr(&start.mesh).hash(&mut h);
    serde_json::to_string(ops).unwrap_or_default().hash(&mut h);
    h.finish()
}

fn bounds(mesh: &kernel_api::TriMesh) -> Option<([f32; 3], [f32; 3])> {
    let mut lo = [f32::MAX; 3];
    let mut hi = [f32::MIN; 3];
    for p in &mesh.positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (!mesh.positions.is_empty()).then_some((lo, hi))
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
            histories.built.insert(done.body, done.key);
            let document = &mut self.session.document;
            let Some(start) = document.mesh_start(done.body).cloned() else {
                continue;
            };
            for id in &done.features {
                document.set_feature_error(*id, None);
            }
            match done.answer {
                Ok(answer) => {
                    if let Some((k, why)) = &answer.failed
                        && let Some(id) = done.features.get(*k)
                    {
                        document.set_feature_error(*id, Some(why.clone()));
                    }
                    let bounds_mm = bounds(&answer.mesh);
                    document.set_imported_geometry(
                        done.body,
                        ImportedGeometry {
                            mesh: Arc::new(answer.mesh),
                            bounds_mm,
                            mesh_path: None,
                            ..start
                        },
                    );
                }
                Err(why) => {
                    if let Some(id) = done.features.first() {
                        document.set_feature_error(*id, Some(why));
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
            let key = key_of(start, &plan.ops);
            let histories = &mut self.session.mesh_histories;
            if histories.built.get(&plan.body) == Some(&key)
                || histories.making.get(&plan.body) == Some(&key)
            {
                continue;
            }
            histories.making.insert(plan.body, key);
            let tx = histories.channel.get_or_insert_with(channel).0.clone();
            let mesh = Arc::clone(&start.mesh);
            let (body, ops, features) = (plan.body, plan.ops, plan.features);
            let spawned = crate::platform::spawn("printcad-mesh-history", move || {
                use kernel_api::KernelQueries;
                let answer = kernel_ogeom::QUERIES
                    .build_mesh_chain(&mesh, &ops)
                    .map_err(|e| e.to_string());
                let _ = tx.send(Landed {
                    body,
                    key,
                    features,
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
            let key = key_of(&start, &[]);
            if self.session.mesh_histories.built.get(&body) == Some(&key) {
                continue;
            }
            self.session.mesh_histories.making.remove(&body);
            self.session.mesh_histories.built.insert(body, key);
            self.session.document.set_imported_geometry(
                body,
                ImportedGeometry {
                    mesh_path: None,
                    ..start
                },
            );
        }
    }
}
