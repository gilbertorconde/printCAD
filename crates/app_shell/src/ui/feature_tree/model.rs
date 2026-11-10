//! The tree's model: rows built from the document, its bodies, features,
//! imported parts, notes, arrays and components, with their problems marked.

use super::*;

impl DocumentTree {
    pub fn build(document: &Document, registry: &DocumentService) -> Self {
        let feature_tree = document.feature_tree();
        let mut roots_by_body: HashMap<Option<BodyId>, Vec<TreeNode>> = HashMap::new();

        // Tip metadata per body: the tip feature's seq bounds the build.
        let tip_seq_by_body: HashMap<BodyId, (FeatureId, u64)> = document
            .bodies()
            .iter()
            .filter_map(|body| {
                let tip = body.tip?;
                let seq = feature_tree.get_node(tip)?.seq;
                Some((body.id, (tip, seq)))
            })
            .collect();

        let parents = tree_parents(document, registry);
        let mut children: HashMap<FeatureId, Vec<FeatureId>> = HashMap::new();
        for (child, parent) in &parents {
            children.entry(*parent).or_default().push(*child);
        }
        for (&id, node) in feature_tree.all_nodes() {
            if !parents.contains_key(&id) {
                let tree_node =
                    build_feature_node(document, node, &children, &tip_seq_by_body, registry);
                roots_by_body.entry(node.body).or_default().push(tree_node);
            }
        }

        // Sort feature roots within each body group by insertion order (the
        // history ordering key; creation times have millisecond ties).
        for nodes in roots_by_body.values_mut() {
            nodes.sort_by_key(|n| (n.seq, n.id));
        }

        // Build body nodes and attach their feature subtrees. Bodies represented
        // in imported-object hierarchy are shown there instead to avoid duplicates.
        let body_nodes: Vec<TreeNode> = document
            .bodies()
            .iter()
            .filter(|body| document.imported_object_for_body(body.id).is_none())
            .map(|body| {
                let mut node = build_body_node(body);
                if let Some(children) = roots_by_body.remove(&Some(body.id)) {
                    node.children = children;
                }
                if let Some(link) = &body.link {
                    let file = std::path::Path::new(&link.path)
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    node.detail = Some(format!("Linked from {file}"));
                    node.tooltip = Some(link.path.clone());
                    node.linked_file = true;
                    if document.link_stale(body.id) {
                        node.error = Some(format!(
                            "{file} has changed since this part was read: Reload from file \
                             takes it as it is now"
                        ));
                    }
                }
                if let Some(source) = body.copy_of.and_then(|s| document.body(s)) {
                    let text = format!("Linked copy of {}", source.name);
                    node.tooltip = Some(text.clone());
                    node.detail = Some(text);
                }
                if let Some((array, instance)) = document.array_instance(body.id) {
                    node.detail = Some(format!(
                        "{}: copy {} of {}",
                        array.name,
                        instance.index,
                        array.count.saturating_sub(1)
                    ));
                    node.tooltip = Some(format!(
                        "A copy {} owns; Edit array changes it, Delete skips it",
                        array.name
                    ));
                } else if let Some(array) = document
                    .arrays_of_source(body.id)
                    .first()
                    .and_then(|id| document.copy_array(*id))
                    && node.detail.is_none()
                {
                    node.detail = Some(format!("Repeated by {}", array.name));
                }
                node
            })
            .collect();

        let mut body_nodes = group_arrays(document, nest_in_components(document, body_nodes));

        for &root in document.imported_object_roots() {
            if let Some(node) = build_imported_node(document, root, &mut roots_by_body) {
                body_nodes.push(node);
            }
        }

        // Any remaining roots without a body (or with unknown body IDs) are appended at the end.
        if let Some(mut doc_level) = roots_by_body.remove(&None) {
            body_nodes.append(&mut doc_level);
        }
        for (_key, mut nodes) in roots_by_body {
            body_nodes.append(&mut nodes);
        }

        attach_links(&mut body_nodes, document, registry);
        attach_notes(&mut body_nodes, document, &mut HashSet::new());
        mark_shape_health(&mut body_nodes, document);
        mark_meshes(&mut body_nodes, document);
        mark_problems(&mut body_nodes, document);

        Self {
            document_label: document.name().to_string(),
            nodes: body_nodes,
        }
    }

    /// The row that stands for `body`, with every row above it, root first.
    ///
    /// A pick in the viewport names a body; the tree is the only thing that
    /// knows which row shows it, since an imported part merged into its
    /// instance answers to the instance's id.
    /// The rows from the top down to `id`'s, `id`'s last.
    pub fn path_to(&self, id: TreeItemId) -> Option<Vec<TreeItemId>> {
        fn walk(nodes: &[TreeNode], id: TreeItemId, trail: &mut Vec<TreeItemId>) -> bool {
            for node in nodes {
                trail.push(node.id);
                if node.id == id || walk(&node.children, id, trail) {
                    return true;
                }
                trail.pop();
            }
            false
        }
        let mut trail = Vec::new();
        walk(&self.nodes, id, &mut trail).then_some(trail)
    }

    pub fn path_to_body(&self, body: BodyId) -> Option<Vec<TreeItemId>> {
        fn walk(nodes: &[TreeNode], body: BodyId, trail: &mut Vec<TreeItemId>) -> bool {
            for node in nodes {
                trail.push(node.id);
                if node.body == Some(body) || walk(&node.children, body, trail) {
                    return true;
                }
                trail.pop();
            }
            false
        }
        let mut trail = Vec::new();
        walk(&self.nodes, body, &mut trail).then_some(trail)
    }

    /// The spelled-out description of a tree item, for the details line.
    pub fn detail_for(&self, id: TreeItemId) -> Option<String> {
        fn find(nodes: &[TreeNode], id: TreeItemId) -> Option<&TreeNode> {
            for node in nodes {
                if node.id == id {
                    return Some(node);
                }
                if let Some(found) = find(&node.children, id) {
                    return Some(found);
                }
            }
            None
        }
        if id == TreeItemId::DocumentRoot {
            return Some("Document".to_string());
        }
        find(&self.nodes, id).and_then(|n| n.detail.clone())
    }

    /// Every row with a problem of its own, in the order the tree draws
    /// them.
    pub fn problems(&self) -> Vec<TreeItemId> {
        fn walk(nodes: &[TreeNode], out: &mut Vec<TreeItemId>) {
            for node in nodes {
                if node.problem.is_some() {
                    out.push(node.id);
                }
                walk(&node.children, out);
            }
        }
        let mut out = Vec::new();
        walk(&self.nodes, &mut out);
        out
    }

    /// Whether any problem is an error rather than a warning.
    pub fn has_errors(&self) -> bool {
        fn walk(nodes: &[TreeNode]) -> bool {
            nodes.iter().any(|n| {
                n.problem
                    .as_ref()
                    .is_some_and(|p| p.tone == ui_kit::widgets::Note::Error)
                    || walk(&n.children)
            })
        }
        walk(&self.nodes)
    }

    /// The problem after `from` in the tree's order, wrapping round to the
    /// first; the first when `from` has none.
    pub fn next_problem(&self, from: Option<TreeItemId>) -> Option<TreeItemId> {
        let problems = self.problems();
        let at = from.and_then(|id| problems.iter().position(|p| *p == id));
        match at {
            Some(i) => problems.get((i + 1) % problems.len()).copied(),
            None => problems.first().copied(),
        }
    }

    pub fn document_label(&self) -> &str {
        &self.document_label
    }

    pub(super) fn nodes(&self) -> &[TreeNode] {
        &self.nodes
    }
}

/// The row each feature sits under: the feature it consumes, as the bench
/// that owns it says, when that is a feature of the same body. A feature
/// left out sits at its body's top level. Every parent stands earlier in
/// build order than its child, so no chain of parents loops.
pub(super) fn tree_parents(
    document: &Document,
    registry: &DocumentService,
) -> HashMap<FeatureId, FeatureId> {
    let feature_tree = document.feature_tree();
    feature_tree
        .all_nodes()
        .filter_map(|(&id, node)| {
            let parent = registry.consumed_feature(document, node)?;
            let of = feature_tree.get_node(parent)?;
            (of.body == node.body && (of.seq, of.id) < (node.seq, id)).then_some((id, parent))
        })
        .collect()
}

pub(super) fn build_feature_node(
    document: &Document,
    node: &FeatureNode,
    nested: &HashMap<FeatureId, Vec<FeatureId>>,
    tip_seq_by_body: &HashMap<BodyId, (FeatureId, u64)>,
    registry: &DocumentService,
) -> TreeNode {
    let feature_tree = document.feature_tree();
    let mut children: Vec<TreeNode> = nested
        .get(&node.id)
        .into_iter()
        .flatten()
        .filter_map(|id| feature_tree.get_node(*id))
        .map(|child| build_feature_node(document, child, nested, tip_seq_by_body, registry))
        .collect();

    children.sort_by_key(|n| (n.seq, n.id));

    let tip = node.body.and_then(|b| tip_seq_by_body.get(&b));
    let is_tip = tip.map(|(id, _)| *id == node.id).unwrap_or(false);
    let after_tip = tip.map(|(_, seq)| node.seq > *seq).unwrap_or(false);

    let info = feature_info(registry, node);
    TreeNode {
        id: TreeItemId::Feature(node.id),
        label: node.name.clone(),
        detail: Some(info.family_label.clone()),
        tooltip: Some(feature_tooltip(node, &info, after_tip)),
        dirty: node.dirty,
        visible: node.visible,
        suppressed: node.suppressed,
        error: node.error.clone().or_else(|| {
            registry
                .feature_info(node)
                .is_none()
                .then(|| FeatureInfo::missing_package(node))
                .flatten()
        }),
        warning: registry.feature_warning(document, Some(&kernel_ogeom::QUERIES), node.id, node),
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        refinable: Vec::new(),
        needs_package: registry
            .feature_info(node)
            .is_none()
            .then(|| FeatureInfo::needed_package(node))
            .flatten()
            .and_then(|(id, repo)| Some((id, repo?))),
        mesh: false,
        is_tip,
        after_tip,
        feature_menu: Some(node.id),
        seq: node.seq,
        children,
        imported_object_id: None,
        body: None,
        icon: info.icon,
        accent_icon: false,
        linked: false,
        linked_file: false,
        frozen: false,
        scan: None,
        textured: false,
        faceted: false,
        status: registry.feature_status(node),
        problem: None,
        history_body: node.body,
    }
}

/// Under every row that stands for a body, a link to each feature of
/// another body the benches say acts on it.
pub(super) fn attach_links(
    nodes: &mut [TreeNode],
    document: &Document,
    registry: &DocumentService,
) {
    for node in nodes {
        attach_links(&mut node.children, document, registry);
        let Some(body) = node.body else {
            continue;
        };
        for id in registry.linked_features(document, body) {
            let Some(feature) = document.get_feature_meta(id) else {
                continue;
            };
            let owner = feature
                .body
                .and_then(|b| document.body(b))
                .map(|b| b.name.clone())
                .unwrap_or_default();
            let info = feature_info(registry, feature);
            node.children.push(TreeNode {
                id: TreeItemId::Feature(id),
                label: format!("{} ({owner})", feature.name),
                detail: Some(format!("{} of {owner}", info.family_label)),
                tooltip: Some(format!("{}, kept with {owner}", feature.name)),
                dirty: false,
                visible: true,
                suppressed: feature.suppressed,
                error: None,
                warning: None,
                defect: false,
                repairable: Vec::new(),
                convertible: Vec::new(),
                refinable: Vec::new(),
                mesh: false,
                needs_package: None,
                is_tip: false,
                after_tip: false,
                feature_menu: Some(id),
                seq: u64::MAX,
                children: Vec::new(),
                imported_object_id: None,
                body: None,
                icon: info.icon,
                accent_icon: false,
                linked: true,
                linked_file: false,
                frozen: false,
                scan: None,
                textured: false,
                faceted: false,
                status: None,
                problem: None,
                history_body: None,
            });
        }
    }
}

/// Under the row that stands for a body, its notes in a group of their
/// own, as an imported model's annotations are.
pub(super) fn attach_notes(
    nodes: &mut [TreeNode],
    document: &Document,
    done: &mut HashSet<BodyId>,
) {
    for node in nodes {
        attach_notes(&mut node.children, document, done);
        let Some(body) = node.body else {
            continue;
        };
        if !matches!(node.id, TreeItemId::Body(_) | TreeItemId::ImportedObject(_))
            || !done.insert(body)
        {
            continue;
        }
        let notes = document.body_notes(body);
        if notes.is_empty() {
            continue;
        }
        let rows: Vec<TreeNode> = notes
            .iter()
            .map(|note| note_row(document, body, note))
            .collect();
        node.children.push(TreeNode {
            id: TreeItemId::Notes(body),
            label: "Annotations".to_string(),
            detail: Some(format!(
                "{} note{} on this body",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            )),
            visible: notes.iter().any(|n| !n.hidden),
            icon: "tree-group",
            children: rows,
            ..build_body_node_blank()
        });
    }
}

/// A note's row: its number and what it shows, crossed out once
/// resolved, with a warning while what it was put on is gone and a
/// question mark while it asks.
pub(super) fn note_row(document: &Document, body: BodyId, note: &core_document::Note) -> TreeNode {
    let shown = document.note_text(note);
    let first = shown.lines().next().unwrap_or_default();
    let mut detail = if shown == note.text {
        format!("Note #{}", note.number)
    } else {
        format!("Note #{}: {}", note.number, note.text)
    };
    if note.author == core_document::NoteAuthor::Agent {
        detail.push_str(", by the agent");
    }
    if let Some(done) = note.resolved.as_deref().filter(|d| !d.is_empty()) {
        detail.push_str(&format!(", resolved: {done}"));
    }
    let lost = document.note_placed(body, note).lost;
    TreeNode {
        id: TreeItemId::Note(note.id),
        label: format!("#{}  {first}", note.number),
        detail: Some(detail),
        tooltip: Some(shown),
        visible: !note.hidden,
        suppressed: note.resolved.is_some(),
        warning: if lost {
            Some("Lost: what it was put on is gone from the part; it stays where it was".into())
        } else if note.asks && note.resolved.is_none() {
            Some("A question waiting for an answer".into())
        } else {
            None
        },
        icon: "info",
        body: Some(body),
        ..build_body_node_blank()
    }
}

/// How a feature row presents: what the bench that claimed its kind says,
/// or the plain fallback for a kind no bench claims.
pub(crate) fn feature_info(registry: &DocumentService, node: &FeatureNode) -> FeatureInfo {
    registry
        .feature_info(node)
        .unwrap_or_else(|| FeatureInfo::unowned(node))
}

/// The rows of an array's copies (or, for a component's array, of the
/// components holding each occurrence's copies) gathered under one row
/// for the array, standing where its first copy would, at every level.
pub(super) fn group_arrays(document: &Document, rows: Vec<TreeNode>) -> Vec<TreeNode> {
    if document.copy_arrays().is_empty() {
        return rows;
    }
    let owner = |row: &TreeNode| match row.id {
        TreeItemId::Body(body) => document
            .array_instance(body)
            .filter(|(a, _)| a.component.is_none())
            .map(|(a, _)| a.id),
        TreeItemId::Component(group) => document
            .copy_arrays()
            .iter()
            .find(|a| a.instances.iter().any(|i| i.group == Some(group)))
            .map(|a| a.id),
        _ => None,
    };
    let mut out: Vec<TreeNode> = Vec::new();
    let mut slots: HashMap<core_document::ArrayId, usize> = HashMap::new();
    for mut row in rows {
        row.children = group_arrays(document, std::mem::take(&mut row.children));
        let Some(array) = owner(&row) else {
            out.push(row);
            continue;
        };
        let slot = *slots.entry(array).or_insert_with(|| {
            let a = document.copy_array(array).expect("the array owns the row");
            out.push(TreeNode {
                id: TreeItemId::Array(array),
                label: a.name.clone(),
                detail: Some(format!(
                    "Array: {} cop{}",
                    a.instances.len(),
                    if a.instances.len() == 1 { "y" } else { "ies" }
                )),
                tooltip: Some(
                    "The copies the array owns; Edit array changes them, Delete takes them \
                     away"
                        .to_string(),
                ),
                visible: false,
                icon: "tree-group",
                ..build_body_node_blank()
            });
            out.len() - 1
        });
        let group = &mut out[slot];
        group.visible |= row.visible;
        group.children.push(row);
    }
    out
}

/// Body rows put in their components' rows, nested as the components
/// are; a component's row stands where its first body would. Components
/// with no body come last in the one they sit in.
pub(super) fn nest_in_components(document: &Document, rows: Vec<TreeNode>) -> Vec<TreeNode> {
    if document.components().is_empty() {
        return rows;
    }
    /// A level's rows, in order: rows themselves, or where a component's
    /// row goes.
    enum Slot {
        Row(Box<TreeNode>),
        Component(ComponentId),
    }
    let mut levels: HashMap<Option<ComponentId>, Vec<Slot>> = HashMap::new();
    let mut placed: HashSet<ComponentId> = HashSet::new();
    fn place(
        document: &Document,
        id: ComponentId,
        levels: &mut HashMap<Option<ComponentId>, Vec<Slot>>,
        placed: &mut HashSet<ComponentId>,
    ) {
        if !placed.insert(id) {
            return;
        }
        let parent = document
            .component(id)
            .and_then(|c| c.parent)
            .filter(|p| document.component(*p).is_some());
        if let Some(parent) = parent {
            place(document, parent, levels, placed);
        }
        levels.entry(parent).or_default().push(Slot::Component(id));
    }
    for row in rows {
        let at = row.body.and_then(|b| document.component_of(b));
        if let Some(component) = at {
            place(document, component, &mut levels, &mut placed);
        }
        levels.entry(at).or_default().push(Slot::Row(Box::new(row)));
    }
    for component in document.components() {
        place(document, component.id, &mut levels, &mut placed);
    }
    fn build(
        document: &Document,
        level: Option<ComponentId>,
        levels: &mut HashMap<Option<ComponentId>, Vec<Slot>>,
        seen: &mut HashSet<ComponentId>,
    ) -> Vec<TreeNode> {
        let slots = levels.remove(&level).unwrap_or_default();
        slots
            .into_iter()
            .filter_map(|slot| match slot {
                Slot::Row(row) => Some(*row),
                Slot::Component(id) => {
                    if !seen.insert(id) {
                        return None;
                    }
                    let component = document.component(id)?;
                    let children = build(document, Some(id), levels, seen);
                    let bodies = document.component_bodies(id);
                    let visible = bodies.is_empty()
                        || bodies
                            .iter()
                            .any(|b| document.body(*b).is_some_and(|x| !x.hidden));
                    let kind = if component.flexible {
                        "Flexible component: the joints inside it move"
                    } else {
                        "Rigid component: moves as one"
                    };
                    Some(TreeNode {
                        id: TreeItemId::Component(id),
                        label: component.name.clone(),
                        detail: Some(kind.to_string()),
                        tooltip: Some(kind.to_string()),
                        visible,
                        icon: "tree-group",
                        accent_icon: true,
                        body: None,
                        children,
                        ..build_body_node_blank()
                    })
                }
            })
            .collect()
    }
    build(document, None, &mut levels, &mut HashSet::new())
}

/// A row with nothing set, for rows built field by field.
pub(super) fn build_body_node_blank() -> TreeNode {
    TreeNode {
        id: TreeItemId::DocumentRoot,
        label: String::new(),
        detail: None,
        tooltip: None,
        dirty: false,
        visible: true,
        suppressed: false,
        error: None,
        warning: None,
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        refinable: Vec::new(),
        needs_package: None,
        mesh: false,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children: Vec::new(),
        imported_object_id: None,
        body: None,
        icon: "tree-group",
        accent_icon: false,
        linked: false,
        linked_file: false,
        frozen: false,
        scan: None,
        textured: false,
        faceted: false,
        status: None,
        problem: None,
        history_body: None,
    }
}

pub(super) fn build_body_node(body: &Body) -> TreeNode {
    TreeNode {
        id: TreeItemId::Body(body.id),
        label: body.name.clone(),
        detail: Some("Body".to_string()),
        tooltip: None,
        dirty: false,
        visible: !body.hidden,
        suppressed: false,
        error: None,
        warning: None,
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        refinable: Vec::new(),
        needs_package: None,
        mesh: false,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children: Vec::new(),
        imported_object_id: None,
        body: Some(body.id),
        icon: "tree-body",
        accent_icon: true,
        linked: false,
        linked_file: false,
        frozen: body.frozen,
        scan: None,
        textured: !body.textures.is_empty(),
        faceted: false,
        status: None,
        problem: None,
        history_body: None,
    }
}

/// Mark the rows whose body the kernel's checker calls broken, and every
/// row above one: imported assemblies start closed, and a defect three
/// levels down would otherwise be out of sight. Returns how many defective
/// bodies lie at or below `nodes`.
pub(super) fn mark_shape_health(nodes: &mut [TreeNode], document: &Document) -> usize {
    let mut total = 0;
    for node in nodes {
        let below = mark_shape_health(&mut node.children, document);
        let own = node.body.and_then(|body| {
            let health = document.body_health(body)?;
            health.is_broken().then_some((body, health))
        });
        let mut repairable: Vec<BodyId> = node
            .children
            .iter()
            .flat_map(|c| c.repairable.iter().copied())
            .collect();
        match own {
            Some((body, health)) => {
                let requested = document.body(body).is_some_and(|b| b.repair_requested);
                let next = if health.repaired {
                    ""
                } else if requested {
                    "\nRepairing…"
                } else {
                    "\nRight-click › Repair shape to run the kernel's repair"
                };
                node.error = Some(format!("{}{next}", health.describe()));
                node.defect = true;
                if document.can_repair(body) {
                    repairable.push(body);
                }
                total += below + 1;
            }
            None if below > 0 => {
                if node.error.is_none() {
                    node.error = Some(format!(
                        "{below} part(s) inside have shape defects the kernel's checker \
                         calls broken"
                    ));
                }
                node.defect = true;
                total += below;
            }
            None => {}
        }
        repairable.sort();
        repairable.dedup();
        node.repairable = repairable;
    }
    total
}

/// Name each row's own problem: a feature that fails, is left unbuilt by
/// one before it, has a formula that fails, reads a feature that fails or
/// needs a package that is not loaded, a bench's warning, a body whose
/// shape is broken or whose linked file has changed. A row above a broken
/// part keeps its danger colour but takes no chip: the problem is the
/// part's.
pub(super) fn mark_problems(nodes: &mut [TreeNode], document: &Document) {
    use ui_kit::widgets::Note;
    for node in nodes {
        mark_problems(&mut node.children, document);
        if node.linked {
            continue;
        }
        let problem = |word, tone, why: &str| {
            Some(Problem {
                word,
                tone,
                why: why.to_string(),
            })
        };
        node.problem = match node.id {
            TreeItemId::Feature(id) => {
                let own = document.get_feature_meta(id).and_then(|f| f.error.clone());
                let failed_input = || {
                    document
                        .feature_tree()
                        .dependencies(id)
                        .into_iter()
                        .filter_map(|dep| document.get_feature_meta(dep))
                        .find(|dep| dep.error.is_some())
                        .map(|dep| {
                            format!(
                                "It reads {}, which fails: {}",
                                dep.name,
                                dep.error.as_deref().unwrap_or_default()
                            )
                        })
                };
                match (&node.error, own) {
                    (Some(why), None) => problem("Needs package", Note::Error, why),
                    (Some(why), Some(_)) if why.starts_with("not built") => {
                        problem("Not built", Note::Error, why)
                    }
                    (Some(why), Some(_)) if document.build_formula_error(id).is_some() => {
                        problem("Formula", Note::Error, why)
                    }
                    (Some(why), Some(_)) => problem("Failed", Note::Error, why),
                    (None, _) => match (failed_input(), &node.warning) {
                        (Some(why), _) => problem("Input broken", Note::Error, &why),
                        (None, Some(why)) => problem("Warning", Note::Warning, why),
                        (None, None) => None,
                    },
                }
            }
            TreeItemId::Note(_) => node.warning.as_deref().and_then(|why| {
                let word = if why.starts_with("Lost") { "Lost" } else { "?" };
                problem(word, Note::Warning, why)
            }),
            TreeItemId::Body(_) | TreeItemId::ImportedObject(_) => {
                let body = node.body;
                let why = node.error.as_deref().unwrap_or_default();
                if body.is_some_and(|b| document.body_health(b).is_some_and(|h| h.is_broken())) {
                    problem("Broken shape", Note::Error, why)
                } else if body.is_some_and(|b| document.link_stale(b)) {
                    problem("Stale link", Note::Error, why)
                } else {
                    None
                }
            }
            _ => None,
        };
    }
}

/// Say which rows are meshes, and gather the ones a row's "Convert to solid"
/// acts on (its own body and every mesh body below it not yet sent) and
/// the converted solids its "Refine shape" acts on.
pub(super) fn mark_meshes(nodes: &mut [TreeNode], document: &Document) {
    for node in nodes {
        mark_meshes(&mut node.children, document);
        let mut convertible: Vec<BodyId> = node
            .children
            .iter()
            .flat_map(|c| c.convertible.iter().copied())
            .collect();
        if let Some(body) = node.body
            && document.is_mesh_body(body)
        {
            let requested = document.body(body).is_some_and(|b| b.solid_requested);
            node.icon = "workbench-mesh";
            node.mesh = true;
            node.detail = Some(if requested {
                "Mesh, converting to a solid".to_string()
            } else {
                "Mesh: draws and measures, takes features once converted to a solid".to_string()
            });
            if document.can_convert(body) {
                convertible.push(body);
            }
        }
        convertible.sort();
        convertible.dedup();
        node.convertible = convertible;
        let mut refinable: Vec<BodyId> = node
            .children
            .iter()
            .flat_map(|c| c.refinable.iter().copied())
            .collect();
        if let Some(body) = node.body
            && document.can_refine(body)
        {
            refinable.push(body);
        }
        refinable.sort();
        refinable.dedup();
        node.refinable = refinable;
        node.faceted = node
            .body
            .and_then(|body| document.body_health(body))
            .is_some_and(|h| h.faceted);
        node.scan = node
            .body
            .filter(|body| document.body_scan(*body).is_some())
            .map(|body| scan_tip(document, body));
    }
}

pub(super) fn kind_word(kind: kernel_api::ImportedNodeKind) -> &'static str {
    match kind {
        kernel_api::ImportedNodeKind::Assembly => "assembly",
        kernel_api::ImportedNodeKind::Part => "part",
        kernel_api::ImportedNodeKind::Instance => "instance",
        kernel_api::ImportedNodeKind::Annotations => "annotations",
        kernel_api::ImportedNodeKind::Annotation => "annotation",
    }
}

/// The icon an imported annotation row shows, by what it states.
pub(super) fn annotation_icon(annotation: Option<&core_document::Annotation>) -> &'static str {
    match annotation.map(|a| a.kind) {
        Some(kernel_api::AnnotationKind::Dimension) => "dimensional-constraint",
        Some(kernel_api::AnnotationKind::Tolerance) => "check-geometry",
        Some(kernel_api::AnnotationKind::Datum) => "datum-plane",
        Some(kernel_api::AnnotationKind::Note) => "info",
        Some(kernel_api::AnnotationKind::Other) | None => "tree-feature",
    }
}

/// The row of imported object `id` and its children; a part's body's
/// features (a history an edit gave it) go under the part's row.
pub(super) fn build_imported_node(
    document: &Document,
    id: Uuid,
    features: &mut HashMap<Option<BodyId>, Vec<TreeNode>>,
) -> Option<TreeNode> {
    let imported = document.imported_object(id)?;

    // An instance whose only child is the product it instances is one thing
    // to the user, not two: show a single row named for the instance, with
    // the product's children hoisted under it. Selection and visibility keep
    // the instance's identity: hiding an instance hides that placement.
    if imported.kind == kernel_api::ImportedNodeKind::Instance
        && imported.children.len() == 1
        && let Some(target) = document.imported_object(imported.children[0])
        && target.kind != kernel_api::ImportedNodeKind::Instance
    {
        let mut merged = build_imported_node(document, target.id, features)?;
        merged.id = TreeItemId::ImportedObject(imported.id);
        merged.imported_object_id = Some(imported.id);
        merged.visible = imported.visible && target.visible;
        let instance_name = imported.name.trim();
        if !instance_name.is_empty() && instance_name != "Instance" {
            merged.label = imported.name.clone();
        }
        merged.detail = Some(format!(
            "Instance of {} {}",
            kind_word(target.kind),
            target.name
        ));
        return Some(merged);
    }

    let mut children = Vec::new();
    for child_id in &imported.children {
        if let Some(child) = build_imported_node(document, *child_id, features) {
            children.push(child);
        }
    }
    if let Some(body) = imported.body_id
        && let Some(history) = features.remove(&Some(body))
    {
        children.extend(history);
    }
    let label = if imported.name.is_empty() {
        "Imported".to_string()
    } else {
        imported.name.clone()
    };
    let detail = match imported.kind {
        kernel_api::ImportedNodeKind::Assembly => "Assembly".to_string(),
        kernel_api::ImportedNodeKind::Part if imported.body_id.is_some() => {
            "Part, linked to a body".to_string()
        }
        kernel_api::ImportedNodeKind::Part => "Part".to_string(),
        kernel_api::ImportedNodeKind::Instance => "Instance".to_string(),
        kernel_api::ImportedNodeKind::Annotations => {
            format!("{} annotations from the file", imported.children.len())
        }
        kernel_api::ImportedNodeKind::Annotation => match &imported.annotation {
            Some(annotation) => format!("{}: {}", annotation.kind.label(), annotation.text),
            None => "Annotation".to_string(),
        },
    };
    Some(TreeNode {
        id: TreeItemId::ImportedObject(imported.id),
        label,
        detail: Some(detail),
        tooltip: match &imported.annotation {
            Some(annotation) => Some(annotation.text.clone()),
            None => imported
                .body_id
                .map(|body| format!("Linked body: {}", body.0)),
        },
        dirty: false,
        visible: imported.visible,
        suppressed: false,
        error: None,
        warning: None,
        defect: false,
        repairable: Vec::new(),
        convertible: Vec::new(),
        refinable: Vec::new(),
        needs_package: None,
        mesh: false,
        is_tip: false,
        after_tip: false,
        feature_menu: None,
        seq: 0,
        children,
        imported_object_id: Some(imported.id),
        body: imported.body_id,
        icon: match imported.kind {
            kernel_api::ImportedNodeKind::Assembly => "tree-group",
            kernel_api::ImportedNodeKind::Part => "tree-body",
            kernel_api::ImportedNodeKind::Instance => "tree-feature",
            kernel_api::ImportedNodeKind::Annotations => "tree-group",
            kernel_api::ImportedNodeKind::Annotation => {
                annotation_icon(imported.annotation.as_ref())
            }
        },
        accent_icon: imported.body_id.is_some(),
        linked: false,
        linked_file: false,
        frozen: false,
        scan: None,
        textured: false,
        faceted: false,
        status: None,
        problem: None,
        history_body: None,
    })
}
