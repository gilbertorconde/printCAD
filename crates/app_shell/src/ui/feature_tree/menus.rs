//! The tree rows' menus: a feature's and a body's.

use super::*;

/// A feature row's menu: its own edits, its place in history, its body's
/// look and placement, the clipboard, its formulas and the rest.
pub(super) fn attach_feature_menu(
    response: Response,
    node: &TreeNode,
    options: &TreeDrawOptions<'_>,
    result: &mut TreeUiResult,
) -> Response {
    let Some(feature_id) = node.feature_menu else {
        return attach_body_menu(response, node, options, result);
    };
    let mut command = None;
    let mut delete = false;
    let mut bench_command = None;
    let mut install = None;
    let document = options.bench_menus.map(|(d, _)| d);
    let body = document
        .and_then(|d| d.get_feature_meta(feature_id))
        .and_then(|n| n.body);
    response.context_menu(|ui| {
        ui_kit::widgets::fitted_menu(ui, |ui| {
            if let Some((id, repo)) = &node.needs_package {
                if ui
                    .button(format!("Install {id}"))
                    .on_hover_text(format!("From github.com/{repo}"))
                    .clicked()
                {
                    install = Some(repo.clone());
                    ui.close();
                }
                ui.separator();
            } else if ui
                .button(egui::RichText::new(format!("Edit {}", node.label)).strong())
                .clicked()
            {
                result.activation = Some(node.id);
                ui.close();
            }
            if menu_entry(ui, "Rename", options.key("edit.rename")).clicked() {
                result.local = Some(super::super::MenuLocal::Rename(node.id));
                ui.close();
            }
            if let Some(body) = body {
                ui.separator();
                if ui
                    .button("Appearance…")
                    .on_hover_text("Its body's colour, see-through, face colours and material")
                    .clicked()
                {
                    result.local = Some(super::super::MenuLocal::Task(
                        super::super::OpenTask::Appearance(body, None),
                    ));
                    ui.close();
                }
                if ui
                    .button("Placement…")
                    .on_hover_text("Move or turn its body by numbers")
                    .clicked()
                {
                    result.local = Some(super::super::MenuLocal::Task(
                        super::super::OpenTask::Placement(body),
                    ));
                    ui.close();
                }
            }
            ui.separator();
            let suppress_label = if node.suppressed {
                "Unsuppress"
            } else {
                "Suppress"
            };
            // With several features selected, it takes them all to this
            // row's opposite state.
            let together = options.acting_on(feature_id);
            if together.len() > 1 {
                let label = format!("{suppress_label} {} features", together.len());
                if ui.button(label).clicked() {
                    result
                        .commands
                        .push(super::super::UiCommand::SuppressFeatures {
                            features: together,
                            suppressed: !node.suppressed,
                        });
                    ui.close();
                }
            } else if ui.button(suppress_label).clicked() {
                command = Some(TreeFeatureCommand::Suppress(!node.suppressed));
                ui.close();
            }
            let visible_label = if node.visible { "Hide" } else { "Show" };
            if menu_entry(ui, visible_label, options.key("view.toggle_visibility")).clicked() {
                command = Some(TreeFeatureCommand::SetVisible(!node.visible));
                ui.close();
            }
            if node.is_tip {
                if ui
                    .button("Clear tip")
                    .on_hover_text("Expose the full history again")
                    .clicked()
                {
                    command = Some(TreeFeatureCommand::ClearTip);
                    ui.close();
                }
            } else if ui
                .button("Set as tip")
                .on_hover_text(
                    "Preview the history up to this feature; later features are excluded",
                )
                .clicked()
            {
                command = Some(TreeFeatureCommand::SetTip);
                ui.close();
            }
            ui.separator();
            if ui
                .button("Move up")
                .on_hover_text("Swap with the previous feature in the build history")
                .clicked()
            {
                command = Some(TreeFeatureCommand::MoveUp);
                ui.close();
            }
            if ui
                .button("Move down")
                .on_hover_text("Swap with the next feature in the build history")
                .clicked()
            {
                command = Some(TreeFeatureCommand::MoveDown);
                ui.close();
            }
            if ui
                .button("Move after…")
                .on_hover_text("Pick where it goes in its body's history, in the task panel")
                .clicked()
            {
                result.local = Some(super::super::MenuLocal::Task(
                    super::super::OpenTask::History(feature_id),
                ));
                ui.close();
            }
            if let (Some(document), Some(body)) = (document, body) {
                let frozen = document.body_frozen(body);
                let label = if frozen { "Thaw body" } else { "Freeze body" };
                if ui
                    .button(label)
                    .on_hover_text("A frozen body keeps its shape; its features are not rebuilt")
                    .clicked()
                {
                    result.commands.push(super::super::UiCommand::BodyEdit {
                        body,
                        edit: super::super::BodyEdit::Frozen(!frozen),
                    });
                    ui.close();
                }
            }
            ui.separator();
            clipboard_entries(ui, options, node.id, result);
            if menu_entry(ui, "Delete", options.key("edit.delete")).clicked() {
                delete = true;
                ui.close();
            }
            if let Some(document) = document {
                formula_entries(ui, document, feature_id, result);
            }
            ui.separator();
            if let Some(body) = body
                && ui
                    .button("Recompute")
                    .on_hover_text("Build the feature's body again from its history")
                    .clicked()
            {
                result.commands.push(super::super::UiCommand::BodyEdit {
                    body,
                    edit: super::super::BodyEdit::Recompute,
                });
                ui.close();
            }
            if ui
                .button("Send to console")
                .on_hover_text("Put the feature in the script console's input")
                .clicked()
            {
                result.local = Some(super::super::MenuLocal::Console(format!(
                    "pc.doc.feature{{id = \"{}\"}}",
                    feature_id.0
                )));
                ui.close();
            }
            if menu_entry(ui, "Properties", options.key("edit.properties")).clicked() {
                result.local = Some(super::super::MenuLocal::Properties(
                    node.id,
                    super::super::property_panel::PropertyTab::Data,
                ));
                ui.close();
            }
            bench_command = bench_menu_entries(ui, options, MenuScope::TreeFeature(feature_id));
        });
    });
    if delete {
        result.delete_item = Some(node.id);
    }
    if let Some(command) = command {
        result.feature_command = Some((feature_id, command));
    }
    if bench_command.is_some() {
        result.bench_command = bench_command;
    }
    if install.is_some() {
        result.install_package = install;
    }
    response
}

/// Cut, Copy and Paste of a row: the row is selected, then the active
/// bench's clipboard acts on it as the Edit menu's would.
pub(super) fn clipboard_entries(
    ui: &mut Ui,
    options: &TreeDrawOptions<'_>,
    item: TreeItemId,
    result: &mut TreeUiResult,
) {
    use super::super::EditCommand;
    for (label, id, command) in [
        ("Cut", "edit.cut", EditCommand::Cut),
        ("Copy", "edit.copy", EditCommand::Copy),
        ("Paste", "edit.paste", EditCommand::Paste),
    ] {
        if menu_entry(ui, label, options.key(id)).clicked() {
            result
                .commands
                .push(super::super::UiCommand::SelectTreeItem(item));
            result.commands.push(super::super::UiCommand::Edit(command));
            ui.close();
        }
    }
}

/// The id egui keeps the copied formulas under.
pub(super) fn formula_clipboard() -> egui::Id {
    egui::Id::new("tree_formula_clipboard")
}

/// Copy a feature's formulas, and paste the ones copied onto it.
pub(super) fn formula_entries(
    ui: &mut Ui,
    document: &Document,
    feature: FeatureId,
    result: &mut TreeUiResult,
) {
    let Some(node) = document.get_feature_meta(feature) else {
        return;
    };
    let copied: Option<std::collections::BTreeMap<String, String>> =
        ui.data(|d| d.get_temp(formula_clipboard()));
    if !node.formulas.is_empty()
        && ui
            .button("Copy formulas")
            .on_hover_text("Keep this feature's formulas to paste on another")
            .clicked()
    {
        let formulas: std::collections::BTreeMap<String, String> = node
            .formulas
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        ui.data_mut(|d| d.insert_temp(formula_clipboard(), formulas));
        ui.close();
    }
    if let Some(formulas) = copied
        && ui
            .button("Paste formulas")
            .on_hover_text(
                "Set the numbers this feature shares with the copied one by the same formulas",
            )
            .clicked()
    {
        result
            .commands
            .push(super::super::UiCommand::PasteFormulas { feature, formulas });
        ui.close();
    }
}

/// A menu entry with its key, if it has one, beside it.
pub(super) fn menu_entry(ui: &mut Ui, label: &str, key: Option<String>) -> Response {
    let mut button = egui::Button::new(label);
    if let Some(key) = key {
        button = button.shortcut_text(key);
    }
    ui.add(button)
}

/// The benches' entries for `scope`, after a separator when there are
/// any. The picked one, if any.
pub(super) fn bench_menu_entries(
    ui: &mut Ui,
    options: &TreeDrawOptions<'_>,
    scope: MenuScope,
) -> Option<(WorkbenchId, String, MenuScope)> {
    let (document, registry) = options.bench_menus?;
    let items = registry.menu_items(&scope, document);
    if items.is_empty() {
        return None;
    }
    ui.separator();
    let mut picked = None;
    for (bench, item) in items {
        if item.separator_before {
            ui.separator();
        }
        let mut button = egui::Button::new(&item.label);
        if let Some(key) = options.key(&item.id) {
            button = button.shortcut_text(key);
        }
        let button = ui.add_enabled(item.enabled, button);
        let button = match &item.hint {
            Some(hint) => button.on_hover_text(hint),
            None => button,
        };
        if button.clicked() {
            picked = Some((bench, item.id, scope.clone()));
            ui.close();
        }
    }
    picked
}

/// The menu of a component row (Take apart, then what the benches offer),
/// or of a body or imported part row: the conversions and repairs its
/// bodies wait for, the linked file, selection, renaming, visibility, the
/// shared body entries, the clipboard, Delete, which takes the body's
/// features and geometry with it, and what the benches offer.
pub(super) fn attach_body_menu(
    response: Response,
    node: &TreeNode,
    options: &TreeDrawOptions<'_>,
    result: &mut TreeUiResult,
) -> Response {
    match node.id {
        TreeItemId::Note(note) => {
            let mut delete = false;
            response.context_menu(|ui| {
                ui_kit::widgets::fitted_menu(ui, |ui| {
                    if ui
                        .button("Edit note…")
                        .on_hover_text("Its text and where its label stands, in the task panel")
                        .clicked()
                    {
                        result.local = Some(super::super::MenuLocal::Task(
                            super::super::OpenTask::EditNote(note),
                        ));
                        ui.close();
                    }
                    if ui
                        .button("Open card")
                        .on_hover_text("The note's card by its point in the view")
                        .clicked()
                    {
                        result
                            .commands
                            .push(super::super::UiCommand::OpenNoteCard(note));
                        ui.close();
                    }
                    let resolved = node.suppressed;
                    if ui
                        .button(if resolved { "Reopen" } else { "Resolve" })
                        .clicked()
                    {
                        result.commands.push(super::super::UiCommand::ResolveNote {
                            note,
                            resolved: (!resolved).then(String::new),
                        });
                        ui.close();
                    }
                    let visible_label = if node.visible { "Hide" } else { "Show" };
                    if menu_entry(ui, visible_label, options.key("view.toggle_visibility"))
                        .clicked()
                    {
                        result
                            .commands
                            .push(super::super::UiCommand::SetNoteVisible {
                                note,
                                visible: !node.visible,
                            });
                        ui.close();
                    }
                    if menu_entry(ui, "Delete", options.key("edit.delete")).clicked() {
                        delete = true;
                        ui.close();
                    }
                });
            });
            if delete {
                result.delete_item = Some(node.id);
            }
            return response;
        }
        TreeItemId::Notes(_) => {
            let mut delete = false;
            response.context_menu(|ui| {
                ui_kit::widgets::fitted_menu(ui, |ui| {
                    if menu_entry(ui, "Delete all notes", options.key("edit.delete")).clicked() {
                        delete = true;
                        ui.close();
                    }
                });
            });
            if delete {
                result.delete_item = Some(node.id);
            }
            return response;
        }
        TreeItemId::Array(array) => {
            let mut delete = false;
            let source = options
                .bench_menus
                .and_then(|(d, _)| d.copy_array(array))
                .map(|a| (a.source, a.component));
            response.context_menu(|ui| {
                ui_kit::widgets::fitted_menu(ui, |ui| {
                    if let Some((body, component)) = source
                        && menu_entry(ui, "Edit array…", None)
                            .on_hover_text("Its kind, count and spacing, in the task panel")
                            .clicked()
                    {
                        let task = match component {
                            Some(component) => super::super::OpenTask::ComponentArray(component),
                            None => super::super::OpenTask::Array(body),
                        };
                        result.local = Some(super::super::MenuLocal::Task(task));
                        ui.close();
                    }
                    if menu_entry(ui, "Delete array", options.key("edit.delete"))
                        .on_hover_text("Every copy it owns goes; what it repeats stays")
                        .clicked()
                    {
                        delete = true;
                        ui.close();
                    }
                });
            });
            if delete {
                result.delete_item = Some(node.id);
            }
            return response;
        }
        _ => {}
    }
    if let TreeItemId::Component(component) = node.id {
        let mut delete = false;
        let mut bench_command = None;
        response.context_menu(|ui| {
            ui_kit::widgets::fitted_menu(ui, |ui| {
                if menu_entry(ui, "Take apart", options.key("edit.delete"))
                    .on_hover_text("Remove the component; its bodies stay, one level up")
                    .clicked()
                {
                    delete = true;
                    ui.close();
                }
                let repeated = options
                    .bench_menus
                    .is_some_and(|(d, _)| d.array_of_component(component).is_some());
                let label = if repeated {
                    "Edit array…"
                } else {
                    "Array of copies…"
                };
                if menu_entry(ui, label, None)
                    .on_hover_text(
                        "Copies of every body in the component, held together, in a row, a \
                         ring, along a path or at points, in the task panel",
                    )
                    .clicked()
                {
                    result.local = Some(super::super::MenuLocal::Task(
                        super::super::OpenTask::ComponentArray(component),
                    ));
                    ui.close();
                }
                bench_command =
                    bench_menu_entries(ui, options, MenuScope::TreeComponent(component));
            });
        });
        if delete {
            result.delete_item = Some(node.id);
        }
        if bench_command.is_some() {
            result.bench_command = bench_command;
        }
        return response;
    }
    if !matches!(node.id, TreeItemId::Body(_) | TreeItemId::ImportedObject(_)) {
        return response;
    }
    let mut select = false;
    let mut delete = false;
    let mut repair = false;
    let mut convert = false;
    let mut bench_command = None;
    response.context_menu(|ui| {
        ui_kit::widgets::fitted_menu(ui, |ui| {
            if !node.convertible.is_empty() {
                let label = match node.convertible.len() {
                    1 => "Convert to solid".to_string(),
                    n => format!("Convert {n} meshes to solids"),
                };
                if ui
                    .button(label)
                    .on_hover_text(
                        "Build a B-rep solid from the mesh's triangles, flat regions merged \
                     into faces and curved ones kept as facets (Refine shape then finds \
                     their round surfaces), so it can be measured, checked and take \
                     features. This clears the undo history.",
                    )
                    .clicked()
                {
                    convert = true;
                    ui.close();
                }
                ui.separator();
            }
            if !node.refinable.is_empty() {
                let label = match node.refinable.len() {
                    1 => "Refine shape".to_string(),
                    n => format!("Refine {n} shapes"),
                };
                if ui
                    .button(label)
                    .on_hover_text(
                        "Find the cylinders, cones, spheres and tori among the converted \
                     solid's facets and rebuild them as round faces, so holes and fillets \
                     measure and take features as drawn ones do. Where the facets do not \
                     fit a surface they stay as they are. This clears the undo history.",
                    )
                    .clicked()
                {
                    result.commands.push(super::super::UiCommand::RefineShapes(
                        node.refinable.clone(),
                    ));
                    ui.close();
                }
                ui.separator();
            }
            if !node.repairable.is_empty() {
                let label = match node.repairable.len() {
                    1 => "Repair shape".to_string(),
                    n => format!("Repair {n} shapes"),
                };
                if ui
                    .button(label)
                    .on_hover_text(
                        "Run the kernel's repair on the shapes its checker calls broken. \
                     This clears the undo history.",
                    )
                    .clicked()
                {
                    repair = true;
                    ui.close();
                }
                ui.separator();
            }
            if let (true, Some(body)) = (node.linked_file, node.body) {
                if ui
                    .button("Reload from file")
                    .on_hover_text("Read the part again from its file as it is now")
                    .clicked()
                {
                    result
                        .commands
                        .push(super::super::UiCommand::ReloadLink(body));
                    ui.close();
                }
                if ui.button("Open the file").clicked() {
                    result
                        .commands
                        .push(super::super::UiCommand::OpenLinkSource(body));
                    ui.close();
                }
                ui.separator();
            }
            if node.body.is_some() {
                if ui
                    .button("Select body")
                    .on_hover_text("Select the whole body in the viewport")
                    .clicked()
                {
                    select = true;
                    ui.close();
                }
                if menu_entry(ui, "Rename", options.key("edit.rename")).clicked() {
                    result.local = Some(super::super::MenuLocal::Rename(node.id));
                    ui.close();
                }
                ui.separator();
                let visible_label = if node.visible { "Hide" } else { "Show" };
                if menu_entry(ui, visible_label, options.key("view.toggle_visibility")).clicked() {
                    if let Some(body) = node.body {
                        result
                            .commands
                            .push(super::super::UiCommand::SetBodyVisible {
                                body,
                                visible: !node.visible,
                            });
                    }
                    ui.close();
                }
                if menu_entry(ui, "Isolate", options.key("view.isolate")).clicked() {
                    result
                        .commands
                        .push(super::super::UiCommand::Isolate(node.body));
                    ui.close();
                }
                if menu_entry(ui, "Hide the others", None).clicked() {
                    result
                        .commands
                        .push(super::super::UiCommand::HideOthers(node.body));
                    ui.close();
                }
                if menu_entry(ui, "Show all", options.key("view.show_all")).clicked() {
                    result.commands.push(super::super::UiCommand::ShowAllBodies);
                    ui.close();
                }
                ui.separator();
            }
            if let (Some(body), Some((document, _))) = (node.body, options.bench_menus) {
                let mut local = None;
                if super::super::body_menu::body_entries(
                    ui,
                    document,
                    body,
                    None,
                    None,
                    &mut result.commands,
                    &mut local,
                ) {
                    ui.close();
                }
                if local.is_some() {
                    result.local = local;
                }
                ui.separator();
            }
            clipboard_entries(ui, options, node.id, result);
            if menu_entry(ui, "Delete", options.key("edit.delete"))
                .on_hover_text("Remove this body, its features and its geometry")
                .clicked()
            {
                delete = true;
                ui.close();
            }
            if let Some(body) = node.body {
                bench_command = bench_menu_entries(ui, options, MenuScope::TreeBody(body));
            }
        });
    });
    if select {
        result.selection = Some(node.id);
    }
    if delete {
        result.delete_item = Some(node.id);
    }
    if repair {
        result.repair = Some(node.repairable.clone());
    }
    if convert {
        result.convert = Some(node.convertible.clone());
    }
    if bench_command.is_some() {
        result.bench_command = bench_command;
    }
    response
}

/// Every body at or below `node`.
pub(super) fn bodies_below(node: &TreeNode, out: &mut Vec<BodyId>) {
    out.extend(node.body);
    for child in &node.children {
        bodies_below(child, out);
    }
}
