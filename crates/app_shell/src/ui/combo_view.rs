//! The left "combo view": the model tree with a filter, the details line,
//! and the active workbench's left-panel content beneath.

use egui::RichText;
use ui_kit::tokens::*;
use ui_kit::widgets::vseparator;
use ui_kit::{sans, sans_medium};

use super::ActiveWorkbench;
use super::feature_tree::{self, TreeItemId};
use super::host_ctx::{HostCtxParams, PanelWriteback, flush_ctx_logs, panel_ctx};
use super::property_panel::{self, PropertyTab};

#[derive(Default)]
pub struct ComboViewResult {
    pub writeback: PanelWriteback,
    pub tree_selection: Option<TreeItemId>,
    pub tree_activation: Option<TreeItemId>,
    pub imported_visibility_change: Option<(uuid::Uuid, bool)>,
    pub tree_feature_command: Option<(core_document::FeatureId, feature_tree::TreeFeatureCommand)>,
    /// The property panel's Label row committed a new name.
    pub rename: Option<(TreeItemId, String)>,
    /// A row the user asked to delete.
    pub delete_item: Option<TreeItemId>,
    /// A bench's own row-menu entry was picked.
    pub bench_command: Option<(core_document::WorkbenchId, String, core_document::MenuScope)>,
    /// The property panel changed a body's look.
    pub body_display: Option<(core_document::BodyId, Option<core_document::BodyDisplay>)>,
}

pub struct ComboViewInputs<'a> {
    pub active_workbench: ActiveWorkbench,
    pub document: &'a mut core_document::Document,
    pub registry: &'a mut core_document::DocumentService,
    pub host: HostCtxParams,
    pub active_tree_selection: Option<TreeItemId>,
    pub active_document_object: Option<core_document::FeatureId>,
    pub editing_feature: Option<core_document::FeatureId>,
    /// A body double-clicked in the viewport: the tree jumps to its row.
    pub reveal_body: Option<core_document::BodyId>,
    /// UI-local substring filter over tree labels.
    pub filter: &'a mut String,
    pub property_tab: &'a mut PropertyTab,
    /// The Label row's in-progress edit.
    pub rename_buffer: &'a mut Option<(TreeItemId, String)>,
}

pub fn draw_combo_view(ui: &mut egui::Ui, inputs: ComboViewInputs<'_>) -> ComboViewResult {
    let ComboViewInputs {
        active_workbench,
        document,
        registry,
        host,
        active_tree_selection,
        active_document_object,
        editing_feature,
        reveal_body,
        filter,
        property_tab,
        rename_buffer,
    } = inputs;
    let mut result = ComboViewResult::default();

    egui::Panel::left("combo_view")
        .resizable(true)
        .default_size(300.0)
        .size_range(240.0..=420.0)
        .frame(egui::Frame::new().fill(BG1))
        .show(ui, |ui| {
            // Nothing paints past the panel, whatever the content's height.
            ui.set_clip_rect(ui.max_rect());
            let rect = ui.max_rect().intersect(ui.clip_rect());
            ui.painter().vline(
                rect.right() - 0.5,
                rect.y_range(),
                egui::Stroke::new(1.0, BORDER),
            );

            // Header: "Model" and the filter box.
            let (header, _) = ui.allocate_exact_size(
                egui::Vec2::new(ui.available_width(), TAB_BAR),
                egui::Sense::hover(),
            );
            ui.painter().hline(
                header.x_range(),
                header.bottom() - 0.5,
                egui::Stroke::new(1.0, BORDER),
            );
            let mut h = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(header.shrink2(egui::Vec2::new(10.0, 0.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            h.spacing_mut().item_spacing.x = SPACE_2;
            ui_kit::icon::draw(&mut h, "tree-group", 14.0, TEXT2);
            h.label(
                RichText::new("Model")
                    .font(sans_medium(FONT_SM))
                    .color(TEXT1),
            );
            h.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // An exact footprint: a frame grown inside the header would
                // take the header's full height.
                let (rect, _) =
                    ui.allocate_exact_size(egui::Vec2::new(104.0, 22.0), egui::Sense::hover());
                ui.painter().rect(
                    rect,
                    4.0,
                    BG2,
                    egui::Stroke::new(1.0, BORDER),
                    egui::StrokeKind::Inside,
                );
                let mut inner = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(rect.shrink2(egui::Vec2::new(6.0, 0.0)))
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                inner.spacing_mut().item_spacing.x = 4.0;
                ui_kit::icon::draw(&mut inner, "search", 11.0, TEXT3);
                inner.add(
                    egui::TextEdit::singleline(filter)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE)
                        .hint_text(RichText::new("Filter").color(TEXT3))
                        .font(sans(FONT_XS)),
                );
            });

            // The active workbench's own panel content.
            if let Ok(wb) = registry.workbench_mut(&active_workbench.0) {
                let mut ctx = panel_ctx(document, host, active_document_object);
                let inner = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        wb.ui_left_panel(ui, &mut ctx);
                    });
                let _ = inner;
                result.writeback = PanelWriteback::take(&mut ctx, active_document_object);
                flush_ctx_logs(&mut ctx);
            }
            ui.add_space(SPACE_1);
            // The tree takes the upper part; the property panel the rest.
            let total = ui.available_height();
            let tree_height = (total * 0.55).max(120.0);
            let mut selected_detail: Option<String> = None;
            let selected_id = active_tree_selection
                .or_else(|| active_document_object.map(TreeItemId::from))
                .unwrap_or(TreeItemId::DocumentRoot);
            egui::ScrollArea::vertical()
                .id_salt("model_tree")
                .max_height(tree_height)
                .min_scrolled_height(tree_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let tree_model = feature_tree::DocumentTree::build(document, registry);
                    let tree_ui = feature_tree::draw_tree(
                        ui,
                        &tree_model,
                        feature_tree::TreeDrawOptions::new(
                            Some(selected_id),
                            editing_feature,
                            filter.trim(),
                        )
                        .revealing(reveal_body)
                        .with_bench_menus(document, registry),
                    );
                    result.bench_command = tree_ui.bench_command;
                    result.tree_selection = tree_ui.selection;
                    result.tree_activation = tree_ui.activation;
                    result.imported_visibility_change = tree_ui.imported_visibility_change;
                    result.tree_feature_command = tree_ui.feature_command;
                    result.delete_item = tree_ui.delete_item;
                    // Hover wins; the selection stands in when the pointer
                    // is elsewhere, so the line never goes blank mid-glance.
                    selected_detail = tree_ui
                        .hovered
                        .and_then(|id| tree_model.detail_for(id))
                        .or_else(|| tree_model.detail_for(selected_id));
                });

            // The tree owns Delete whenever nothing is being typed and no
            // sketch is open — an open sketch keeps it for its geometry.
            let deletable = matches!(
                selected_id,
                TreeItemId::Feature(_) | TreeItemId::Body(_) | TreeItemId::ImportedObject(_)
            );
            if deletable
                && editing_feature.is_none()
                && !ui.ctx().egui_wants_keyboard_input()
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete))
            {
                result.delete_item = Some(selected_id);
            }

            let props = property_panel::draw_property_panel(
                ui,
                document,
                registry,
                selected_id,
                selected_detail.as_deref(),
                property_tab,
                rename_buffer,
            );
            if props.feature_command.is_some() {
                result.tree_feature_command = props.feature_command;
            }
            if props.imported_visibility.is_some() {
                result.imported_visibility_change = props.imported_visibility;
            }
            if props.body_display.is_some() {
                result.body_display = props.body_display;
            }
            result.rename = props.rename;
            let _ = vseparator;
        });

    result
}
