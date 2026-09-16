//! The right "Task" panel: hosts the active workbench's edit session with
//! a header, an OK/Cancel (or Close) strip, and the body the workbench
//! draws. Workbenches that still expose the older right-panel hook are
//! hosted the same way with a Close button.

use core_document::{TaskInfo, TaskOutcome, TaskRequest};
use egui::{Key, Modifiers, RichText, Vec2};
use ui_kit::sans_medium;
use ui_kit::tokens::*;
use ui_kit::widgets::{mono_label, primary_button, secondary_button};

use super::ActiveWorkbench;
use super::host_ctx::{HostCtxParams, PanelWriteback, flush_ctx_logs, panel_ctx};

pub const TASK_PANEL_WIDTH: f32 = 300.0;

#[derive(Default)]
pub struct TaskPanelResult {
    pub writeback: PanelWriteback,
    /// The task closed this frame.
    pub outcome: Option<TaskOutcome>,
    /// A task is open after this frame.
    pub open: bool,
}

pub struct TaskPanelInputs<'a> {
    pub active_workbench: ActiveWorkbench,
    pub document: &'a mut core_document::Document,
    pub registry: &'a mut core_document::DocumentService,
    pub host: HostCtxParams,
    pub active_document_object: Option<core_document::FeatureId>,
    pub task: Option<&'a TaskInfo>,
}

pub fn draw_task_panel(ui: &mut egui::Ui, inputs: TaskPanelInputs<'_>) -> TaskPanelResult {
    let TaskPanelInputs {
        active_workbench,
        document,
        registry,
        host,
        active_document_object,
        task,
    } = inputs;
    let mut result = TaskPanelResult::default();

    let Ok(wb) = registry.workbench_mut(&active_workbench.0) else {
        return result;
    };
    let legacy = task.is_none() && wb.wants_right_panel();
    if task.is_none() && !legacy {
        return result;
    }
    result.open = true;

    // Enter and Escape reach the task only when no text field owns them;
    // a focused field keeps its own Enter/Esc and the next press arrives.
    let no_focus = ui.ctx().memory(|m| m.focused()).is_none();
    let mut request = TaskRequest::default();
    if no_focus {
        ui.ctx().input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                request.accept = true;
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                request.cancel = true;
            }
        });
    }

    egui::Panel::right("task_panel")
        .exact_size(TASK_PANEL_WIDTH)
        .resizable(false)
        .frame(egui::Frame::new().fill(BG1))
        .show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter().vline(
                rect.left() + 0.5,
                rect.y_range(),
                egui::Stroke::new(1.0, BORDER),
            );

            // Header.
            let (header, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width(), TAB_BAR),
                egui::Sense::hover(),
            );
            ui.painter().hline(
                header.x_range(),
                header.bottom() - 0.5,
                egui::Stroke::new(1.0, BORDER),
            );
            let mut h = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(header.shrink2(Vec2::new(10.0, 0.0)))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            h.spacing_mut().item_spacing.x = SPACE_2;
            let (dot, _) = h.allocate_exact_size(Vec2::splat(6.0), egui::Sense::hover());
            h.painter().circle_filled(dot.center(), 3.0, ACCENT);
            h.label(
                RichText::new("Task")
                    .font(sans_medium(FONT_SM))
                    .color(TEXT1),
            );
            let title = task.map(|t| t.title.as_str()).unwrap_or("");
            mono_label(&mut h, title, FONT_XS, TEXT3);
            h.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui_kit::icon::draw(ui, "close", 14.0, TEXT2)
                    .interact(egui::Sense::click())
                    .on_hover_text("Close")
                    .clicked()
                {
                    request.accept = true;
                }
            });

            // Button strip.
            let confirmable = task.is_some_and(|t| t.confirmable);
            egui::Frame::new()
                .fill(BG2)
                .inner_margin(egui::Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let width = ui.available_width();
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_2;
                        if confirmable {
                            let half = (width - SPACE_2) / 2.0;
                            ui.allocate_ui_with_layout(
                                Vec2::new(half, 28.0),
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                |ui| {
                                    if primary_button(ui, "OK").clicked() {
                                        request.accept = true;
                                    }
                                },
                            );
                            ui.allocate_ui_with_layout(
                                Vec2::new(half, 28.0),
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                |ui| {
                                    if secondary_button(ui, "Cancel").clicked() {
                                        request.cancel = true;
                                    }
                                },
                            );
                        } else {
                            ui.allocate_ui_with_layout(
                                Vec2::new(width, 28.0),
                                egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                                |ui| {
                                    if secondary_button(ui, "Close").clicked() {
                                        request.accept = true;
                                    }
                                },
                            );
                        }
                    });
                });
            let strip = ui.min_rect();
            ui.painter().hline(
                strip.x_range(),
                strip.bottom() - 0.5,
                egui::Stroke::new(1.0, BORDER),
            );

            egui::ScrollArea::vertical()
                .id_salt("task_body")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new()
                        .inner_margin(egui::Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            let mut ctx = panel_ctx(document, host, active_document_object);
                            if legacy {
                                wb.ui_right_panel(ui, &mut ctx);
                                if request.accept || request.cancel {
                                    ctx.finish_sketch_requested = true;
                                }
                            } else {
                                match wb.ui_task_panel(ui, &mut ctx, request) {
                                    TaskOutcome::Open => {}
                                    outcome => {
                                        result.outcome = Some(outcome);
                                        result.open = false;
                                    }
                                }
                            }
                            result.writeback =
                                PanelWriteback::take(&mut ctx, active_document_object);
                            flush_ctx_logs(&mut ctx);
                        });
                });
        });

    result
}
