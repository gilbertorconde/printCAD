//! Recording, inverting and applying document ops.

use super::*;

impl Document {
    /// Record a resolved op into the outbox and apply it. The single path
    /// every user-edit mutator funnels through; replay and live edits run
    /// the same `apply_op` code. The inverse is computed from the state the
    /// op is ABOUT to change, so per-user undo can restore it later without
    /// ever snapshotting the document.
    pub(crate) fn record_and_apply(&mut self, operation: op::DocumentOp) {
        if !self.history_suppressed {
            let inverse = self.invert_op(&operation);
            self.journal_pending.record(&operation, inverse);
        }
        self.apply_op(&operation);
        self.pending_ops.record(operation);
    }

    /// Apply an op produced by history traversal (undo/redo): the effect,
    /// this replica's dirty-marking consequences, and the outbox (peers
    /// hear an undo as ordinary ops), but never the journal, which is being
    /// walked, not written.
    pub(crate) fn apply_history_op(&mut self, operation: &op::DocumentOp) {
        self.apply_remote_op(operation);
        self.pending_ops.record(operation.clone());
    }

    /// The op that would restore the state `operation` is about to change.
    /// `None` marks a history barrier: the op is not invertible (imports,
    /// asset registration) and undo history clears rather than lie.
    pub(crate) fn invert_op(&self, operation: &op::DocumentOp) -> Option<op::DocumentOp> {
        use op::DocumentOp as Op;
        Some(match operation {
            Op::SetDocumentName { .. } => Op::SetDocumentName {
                name: self.metadata.name.clone(),
            },
            Op::SetDisplayUnit { .. } => Op::SetDisplayUnit {
                unit: self.display_unit,
            },
            Op::SetMeasureUnit { .. } => Op::SetMeasureUnit {
                unit: self.measure_unit,
            },
            Op::SetAgentRules { .. } => Op::SetAgentRules {
                rules: self.agent_rules.clone(),
            },
            Op::SetSavedSections { .. } => Op::SetSavedSections {
                sections: self.saved_sections.clone(),
            },
            Op::CreateBody { id, .. } => Op::RemoveBody { id: *id },
            Op::CreateLinkedCopy { id, .. } => Op::RemoveBody { id: *id },
            Op::CreateLinkedBody { id, .. } => Op::RemoveBody { id: *id },
            Op::SetBodyLink { id, .. } => Op::SetBodyLink {
                id: *id,
                link: self.body(*id)?.link.clone()?,
            },
            // A body as it was made, with nothing on it, comes back as it
            // was made; one that carried anything cannot.
            Op::RemoveBody { id } => {
                let body = self.body(*id)?;
                if !self.body_is_bare(body) || !self.arrays_of_source(*id).is_empty() {
                    return None;
                }
                Op::CreateBody {
                    id: *id,
                    name: body.name.clone(),
                    created_at: body.created_at,
                }
            }
            Op::SetCopyArray { id, .. } => Op::SetCopyArray {
                id: *id,
                array: self.copy_array(*id).cloned(),
            },
            Op::SetComponent { id, .. } => Op::SetComponent {
                id: *id,
                component: self.component(*id).cloned(),
            },
            Op::SetBodyBase { id, .. } => Op::SetBodyBase {
                id: *id,
                on: self.base_solids.contains_key(id),
            },
            Op::SetMeshStart { id, .. } => Op::SetMeshStart {
                id: *id,
                on: self.mesh_starts.contains_key(id),
            },
            Op::SetBodyShading { id, .. } => Op::SetBodyShading {
                id: *id,
                shading: self.body(*id)?.shading,
            },
            Op::SetBodyFrozen { id, .. } => Op::SetBodyFrozen {
                id: *id,
                frozen: self.body(*id)?.frozen,
            },
            Op::SetBodyScan { id, .. } => Op::SetBodyScan {
                id: *id,
                scan: self.body(*id)?.scan,
            },
            Op::SetBodySelectable { id, .. } => Op::SetBodySelectable {
                id: *id,
                selectable: !self.body(*id)?.unselectable,
            },
            Op::SetBodyTextures { id, .. } => Op::SetBodyTextures {
                id: *id,
                textures: self.body(*id)?.textures.clone(),
            },
            Op::SetBodyNotes { id, .. } => Op::SetBodyNotes {
                id: *id,
                notes: self.body(*id)?.notes.clone(),
                numbered: self.notes_numbered,
            },
            Op::SetBodyMaterial { id, .. } => Op::SetBodyMaterial {
                id: *id,
                material: self.body(*id)?.material.clone(),
            },
            Op::SetFaceColor { id, face, .. } => Op::SetFaceColor {
                id: *id,
                face: *face,
                color: self
                    .body(*id)?
                    .face_colors
                    .iter()
                    .find(|c| c.face == *face)
                    .map(|c| c.color),
            },
            Op::SetBodyComponent { id, .. } => Op::SetBodyComponent {
                id: *id,
                component: self.body(*id)?.component,
            },
            Op::RenameBody { id, .. } => Op::RenameBody {
                id: *id,
                name: self.body(*id)?.name.clone(),
            },
            Op::SetBodyDisplay { id, .. } => Op::SetBodyDisplay {
                id: *id,
                display: self.body(*id)?.display,
            },
            Op::SetBodyVisible { id, .. } => Op::SetBodyVisible {
                id: *id,
                visible: !self.body(*id)?.hidden,
            },
            Op::SetBodyPlacement { id, .. } => Op::SetBodyPlacement {
                id: *id,
                placement: self.body(*id)?.placement,
            },
            Op::SetBodyTip { id, .. } => Op::SetBodyTip {
                id: *id,
                tip: self.body(*id)?.tip,
            },
            Op::AddFeature { id, .. } => Op::RemoveFeature { id: *id },
            Op::UpdateFeatureData { id, .. } => Op::UpdateFeatureData {
                id: *id,
                data: self.feature_tree.get_node(*id)?.data.clone(),
            },
            Op::RenameFeature { id, .. } => Op::RenameFeature {
                id: *id,
                name: self.feature_tree.get_node(*id)?.name.clone(),
            },
            Op::SetFeatureOrigin { id, .. } => {
                let node = self.feature_tree.get_node(*id)?;
                Op::SetFeatureOrigin {
                    id: *id,
                    made_by: node.made_by.clone(),
                    package_source: node.package_source.clone(),
                }
            }
            Op::SetFeatureFormula { id, key, .. } => Op::SetFeatureFormula {
                id: *id,
                key: key.clone(),
                formula: self.feature_tree.get_node(*id)?.formulas.get(key).cloned(),
            },
            Op::SetFeatureVisible { id, .. } => Op::SetFeatureVisible {
                id: *id,
                visible: self.feature_tree.get_node(*id)?.visible,
            },
            Op::SetFeatureSuppressed { id, .. } => Op::SetFeatureSuppressed {
                id: *id,
                suppressed: self.feature_tree.get_node(*id)?.suppressed,
            },
            Op::SetFeatureDependencies { id, .. } => Op::SetFeatureDependencies {
                id: *id,
                deps: self.feature_tree.dependencies(*id),
            },
            Op::SwapFeatureSeq { a, b } => Op::SwapFeatureSeq { a: *a, b: *b },
            Op::SetFeatureBody { id, .. } => {
                let node = self.feature_tree.get_node(*id)?;
                Op::SetFeatureBody {
                    id: *id,
                    body: node.body,
                    seq: node.seq,
                }
            }
            Op::RemoveFeature { id } => {
                let node = self.feature_tree.get_node(*id)?;
                Op::AddFeature {
                    id: node.id,
                    workbench_id: node.workbench_id.clone(),
                    name: node.name.clone(),
                    body: node.body,
                    deps: self.feature_tree.dependencies(*id),
                    data: node.data.clone(),
                    seq: node.seq,
                    created_at: node.created_at,
                    formulas: node.formulas.clone(),
                    made_by: node.made_by.clone(),
                    package_source: node.package_source.clone(),
                }
            }
            Op::SetImportedObjectVisibility { id, .. } => Op::SetImportedObjectVisibility {
                id: *id,
                visible: self.imported_objects.get(id)?.visible,
            },
            // History barriers: an import (or raw graph write) is not worth
            // lying about: clearing undo beats a wrong inverse.
            Op::AddAsset { .. }
            | Op::RequestBodyRepair { .. }
            | Op::ReplaceBodyShape { .. }
            | Op::SetBodyStandIn { .. }
            | Op::RequestMeshSolid { .. }
            | Op::RequestBodyRefine { .. }
            | Op::ImportModel { .. }
            | Op::AppendImportedObjectGraph { .. }
            | Op::ClearImportedObjectGraph => return None,
        })
    }

    /// Ops captured since the last take. Drained once per frame by the host
    /// and handed to the document server.
    pub fn take_pending_ops(&mut self) -> Vec<op::DocumentOp> {
        self.pending_ops.take()
    }

    /// Journal material since the last boundary: (op, inverse) pairs and
    /// whether a non-invertible op crossed. Consumed by the op journal at
    /// gesture boundaries (mouse-up, explicit commits).
    pub fn take_journal_pairs(&mut self) -> (Vec<(op::DocumentOp, op::DocumentOp)>, bool) {
        self.journal_pending.take()
    }

    /// Run `f` with journal capture off: history traversal and remote
    /// application drive the document without journaling themselves.
    pub(crate) fn without_journal<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let previous = self.history_suppressed;
        self.history_suppressed = true;
        let result = f(self);
        self.history_suppressed = previous;
        result
    }

    /// Apply a resolved op **without recording it**: the path a remote or
    /// replayed op takes. Every effect here must be a pure function of
    /// (current state, op); anything nondeterministic was resolved into the
    /// op at capture. Dirty-marking is apply-side policy: applying an op
    /// that changes build inputs marks the affected features dirty, which is
    /// what triggers this replica's own recompute.
    pub fn apply_op(&mut self, operation: &op::DocumentOp) {
        self.apply_op_itself(operation);
        // The bodies the configuration in effect leaves out follow its
        // table.
        let touches_table = match operation {
            op::DocumentOp::AddFeature { workbench_id, .. } => {
                workbench_id.as_str() == configurations::CONFIGURATIONS_KIND
            }
            op::DocumentOp::UpdateFeatureData { id, .. } => self
                .feature_tree
                .get_node(*id)
                .is_some_and(|n| n.workbench_id.as_str() == configurations::CONFIGURATIONS_KIND),
            op::DocumentOp::RemoveFeature { .. } | op::DocumentOp::RemoveBody { .. } => true,
            _ => false,
        };
        if touches_table {
            self.refresh_left_out();
        }
    }

    /// The bodies the configuration in effect leaves out, read again.
    pub(crate) fn refresh_left_out(&mut self) {
        self.left_out = self
            .configurations()
            .map(|(_, t)| t.left_out())
            .unwrap_or_default();
    }

    fn apply_op_itself(&mut self, operation: &op::DocumentOp) {
        use op::DocumentOp as Op;
        match operation {
            Op::SetDocumentName { name } => {
                self.metadata.name.clone_from(name);
            }
            Op::SetDisplayUnit { unit } => {
                self.display_unit = *unit;
            }
            Op::SetMeasureUnit { unit } => {
                self.measure_unit = *unit;
            }
            Op::SetAgentRules { rules } => {
                self.agent_rules.clone_from(rules);
            }
            Op::SetSavedSections { sections } => {
                self.saved_sections.clone_from(sections);
            }
            Op::CreateBody {
                id,
                name,
                created_at,
            } => {
                self.bodies.push(Body::new(*id, name.clone(), *created_at));
            }
            Op::CreateLinkedCopy {
                id,
                name,
                created_at,
                source,
                mirror,
            } => {
                self.bodies.push(Body {
                    copy_of: Some(*source),
                    mirror: *mirror,
                    ..Body::new(*id, name.clone(), *created_at)
                });
                self.refresh_copy(*id);
            }
            Op::CreateLinkedBody {
                id,
                name,
                created_at,
                link,
            } => {
                self.bodies.push(Body {
                    link: Some(link.clone()),
                    ..Body::new(*id, name.clone(), *created_at)
                });
            }
            Op::SetBodyLink { id, link } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.link = Some(link.clone());
                }
                // Read again from the file as it stands now.
                self.imported_meshes.remove(id);
                self.local_meshes.remove(id);
                self.imported_brep_blobs.remove(id);
                self.imported_brep_face_colors.remove(id);
                self.link_stale.remove(id);
                self.refresh_copies_of(*id);
            }
            Op::SetComponent { id, component } => {
                self.apply_set_component(*id, component.as_ref());
            }
            Op::SetCopyArray { id, array } => {
                self.apply_set_copy_array(*id, array.as_ref());
            }
            Op::SetBodyFrozen { id, frozen } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.frozen = *frozen;
                }
            }
            Op::SetBodyShading { id, shading } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.shading = *shading;
                }
            }
            Op::SetBodyScan { id, scan } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.scan = *scan;
                }
            }
            Op::SetBodyBase { id, on } => self.apply_set_body_base(*id, *on),
            Op::SetMeshStart { id, on } => self.apply_set_mesh_start(*id, *on),
            Op::SetBodySelectable { id, selectable } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.unselectable = !selectable;
                }
            }
            Op::SetBodyTextures { id, textures } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.textures.clone_from(textures);
                }
            }
            Op::SetBodyNotes {
                id,
                notes,
                numbered,
            } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.notes.clone_from(notes);
                }
                let highest = notes.iter().map(|n| n.number).max().unwrap_or(0);
                self.notes_numbered = (*numbered).max(highest);
            }
            Op::SetBodyMaterial { id, material } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.material.clone_from(material);
                }
            }
            Op::SetFaceColor { id, face, color } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.face_colors.retain(|c| c.face != *face);
                    if let Some(color) = color {
                        entry.face_colors.push(FaceColor {
                            face: *face,
                            color: *color,
                        });
                    }
                }
            }
            Op::SetBodyComponent { id, component } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.component = *component;
                }
            }
            Op::RenameBody { id, name } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.name.clone_from(name);
                }
            }
            Op::SetBodyDisplay { id, display } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.display = *display;
                }
            }
            Op::RequestBodyRepair { id } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.repair_requested = true;
                }
            }
            Op::ReplaceBodyShape { id, asset, bytes } => {
                self.assets.insert(asset.id, asset.clone());
                self.asset_blobs
                    .insert(asset.id, std::sync::Arc::clone(&bytes.0));
                if let Some(entry) = self.body_mut(*id) {
                    entry.shape_asset = Some(asset.id);
                    // A new shape is checked, repaired and refined afresh.
                    entry.repair_requested = false;
                    entry.refine_requested = false;
                }
            }
            Op::SetBodyStandIn {
                id,
                feature,
                asset,
                bytes,
            } => {
                self.assets.insert(asset.id, asset.clone());
                self.asset_blobs
                    .insert(asset.id, std::sync::Arc::clone(&bytes.0));
                if let Some(entry) = self.body_mut(*id) {
                    entry.stand_in = Some(StandIn {
                        feature: *feature,
                        asset: asset.id,
                    });
                }
            }
            Op::SetBodyVisible { id, visible } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.hidden = !visible;
                }
            }
            Op::SetBodyPlacement { id, placement } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.placement = *placement;
                }
                self.place_geometry(*id);
                // An array's copies stand where it puts them from its
                // source.
                for array in self.arrays_of_source(*id) {
                    self.place_array(array);
                }
            }
            Op::RequestMeshSolid { id } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.solid_requested = true;
                }
            }
            Op::RequestBodyRefine { id } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.refine_requested = true;
                }
            }
            Op::RemoveBody { id } => {
                self.drop_body(*id);
                self.arrays_lose_body(*id);
            }
            Op::SetBodyTip { id, tip } => {
                if let Some(entry) = self.body_mut(*id) {
                    entry.tip = *tip;
                }
            }
            Op::AddFeature {
                id,
                workbench_id,
                name,
                body,
                deps,
                data,
                seq,
                created_at,
                formulas,
                made_by,
                package_source,
            } => {
                self.feature_tree.add_node(FeatureNode {
                    id: *id,
                    workbench_id: workbench_id.clone(),
                    name: name.clone(),
                    body: *body,
                    visible: true,
                    suppressed: false,
                    dirty: false,
                    created_at: *created_at,
                    seq: *seq,
                    error: None,
                    refused_edges: Default::default(),
                    warning: None,
                    data: data.clone(),
                    formulas: formulas.clone(),
                    made_by: made_by.clone(),
                    package_source: package_source.clone(),
                });
                for dep in deps {
                    self.feature_tree.add_dependency(*id, *dep);
                }
            }
            Op::UpdateFeatureData { id, data } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.data = data.clone();
                }
            }
            Op::RenameFeature { id, name } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.name.clone_from(name);
                }
            }
            Op::SetFeatureOrigin {
                id,
                made_by,
                package_source,
            } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.made_by.clone_from(made_by);
                    node.package_source.clone_from(package_source);
                }
            }
            Op::SetFeatureFormula { id, key, formula } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    match formula {
                        Some(formula) => {
                            node.formulas.insert(key.clone(), formula.clone());
                        }
                        None => {
                            node.formulas.remove(key);
                        }
                    }
                }
            }
            Op::SetFeatureVisible { id, visible } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.visible = *visible;
                }
            }
            Op::SetFeatureSuppressed { id, suppressed } => {
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.suppressed = *suppressed;
                }
            }
            Op::SetFeatureDependencies { id, deps } => {
                self.feature_tree.set_dependencies(*id, deps.clone());
                self.feature_tree.mark_dirty(*id);
            }
            Op::SwapFeatureSeq { a, b } => {
                let (Some(seq_a), Some(seq_b)) = (
                    self.feature_tree.get_node(*a).map(|n| n.seq),
                    self.feature_tree.get_node(*b).map(|n| n.seq),
                ) else {
                    return;
                };
                if let Some(n) = self.feature_tree.get_node_mut(*a) {
                    n.seq = seq_b;
                }
                if let Some(n) = self.feature_tree.get_node_mut(*b) {
                    n.seq = seq_a;
                }
                // Order changes results: rebuild the whole history.
                self.feature_tree.mark_dirty(*a);
                self.feature_tree.mark_dirty(*b);
            }
            Op::SetFeatureBody { id, body, seq } => {
                let Some(from) = self.feature_tree.get_node(*id).map(|n| n.body) else {
                    return;
                };
                if let Some(node) = self.feature_tree.get_node_mut(*id) {
                    node.body = *body;
                    node.seq = *seq;
                }
                // Both histories change: rebuild every feature of either.
                let touched: Vec<FeatureId> = self
                    .feature_tree
                    .all_nodes()
                    .filter(|(_, n)| n.body.is_some() && (n.body == from || n.body == *body))
                    .map(|(fid, _)| *fid)
                    .collect();
                for fid in touched {
                    self.feature_tree.mark_dirty(fid);
                }
            }
            Op::RemoveFeature { id } => {
                for dep in self.feature_tree.dependents(*id) {
                    self.feature_tree.mark_dirty(dep);
                }
                self.feature_tree.remove_node(*id);
            }
            Op::AddAsset { asset, bytes } => {
                self.assets.insert(asset.id, asset.clone());
                if let Some(payload) = bytes {
                    self.asset_blobs
                        .insert(asset.id, std::sync::Arc::clone(&payload.0));
                }
            }
            Op::ImportModel {
                asset,
                bytes,
                detail: _,
                bodies,
                roots,
                nodes,
                display_unit,
            } => {
                self.assets.insert(asset.id, asset.clone());
                self.asset_blobs
                    .insert(asset.id, std::sync::Arc::clone(&bytes.0));
                for init in bodies {
                    self.bodies
                        .push(Body::new(init.id, init.name.clone(), init.created_at));
                }
                self.imported_object_roots.extend(roots.iter().copied());
                for node in nodes {
                    self.imported_objects.insert(node.id, node.clone());
                }
                self.rebuild_imported_body_index();
                if let Some(unit) = display_unit {
                    self.display_unit = *unit;
                }
            }
            Op::AppendImportedObjectGraph { roots, nodes } => {
                self.imported_object_roots.extend(roots.iter().copied());
                for node in nodes {
                    self.imported_objects.insert(node.id, node.clone());
                }
                self.rebuild_imported_body_index();
            }
            Op::SetImportedObjectVisibility { id, visible } => {
                if let Some(node) = self.imported_objects.get_mut(id) {
                    node.visible = *visible;
                }
            }
            Op::ClearImportedObjectGraph => {
                self.imported_object_roots.clear();
                self.imported_objects.clear();
                self.imported_body_to_object.clear();
            }
        }
        self.mark_dirty();
    }

    /// Apply a peer's op: the effect plus this replica's own consequences.
    ///
    /// `apply_op` is the pure effect; on top of it, a foreign edit that
    /// changes build inputs must mark the affected features dirty HERE,
    /// because this replica is the one that has to re-derive the geometry
    /// the peer's edit invalidated. (The peer marked its own copy dirty at
    /// capture; dirty flags are per-replica, never part of the op.)
    pub fn apply_remote_op(&mut self, operation: &op::DocumentOp) {
        use op::DocumentOp as Op;
        self.apply_op(operation);
        match operation {
            Op::AddFeature { id, .. }
            | Op::UpdateFeatureData { id, .. }
            | Op::SetFeatureSuppressed { id, .. } => {
                self.mark_feature_dirty(*id);
            }
            Op::SetBodyTip { tip: Some(tip), .. } => {
                self.mark_feature_dirty(*tip);
            }
            _ => {}
        }
    }

    /// The state that must converge across replicas: the serialized document
    /// minus per-replica derivations: dirty flags, recompute errors,
    /// revision history, and the imported-geometry sidecars, whose meshes
    /// are re-derived from asset bytes on each replica. Determinism tests
    /// compare projections, not raw serializations.
    pub fn replicated_projection(&self) -> serde_json::Value {
        let mut value = serde_json::to_value(self).expect("document serializes");
        fn strip_key_recursively(value: &mut serde_json::Value, key: &str) {
            match value {
                serde_json::Value::Object(map) => {
                    map.remove(key);
                    for child in map.values_mut() {
                        strip_key_recursively(child, key);
                    }
                }
                serde_json::Value::Array(items) => {
                    for child in items.iter_mut() {
                        strip_key_recursively(child, key);
                    }
                }
                _ => {}
            }
        }
        strip_key_recursively(&mut value, "dirty");
        if let Some(map) = value.as_object_mut() {
            map.remove("history");
            map.remove("imported_meshes");
            if let Some(meta) = map.get_mut("metadata").and_then(|m| m.as_object_mut()) {
                meta.remove("revision");
            }
        }
        value
    }
}
