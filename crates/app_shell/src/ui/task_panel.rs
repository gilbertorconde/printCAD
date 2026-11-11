//! The right "Task" panel: hosts the active workbench's edit session, or
//! while it has none the application's own task (placement, appearance, a
//! feature's place in history), under one header (the task's name, what it
//! edits, its state as a pill, Cancel and Done) and above the body the
//! task draws.

use core_document::{StatusTone, TaskInfo, TaskOutcome, TaskRequest};
use egui::{Key, Modifiers, RichText};
use ui_kit::tokens::*;
use ui_kit::widgets::{Note, primary_button, secondary_button, status_pill};
use ui_kit::{sans, sans_semibold};

use super::ActiveWorkbench;
use super::host_ctx::{HostCtxParams, PanelWriteback, flush_ctx_logs, panel_ctx};

pub const TASK_PANEL_WIDTH: f32 = 300.0;

#[derive(Default)]
pub struct TaskPanelResult {
    pub writeback: PanelWriteback,
    /// The task closed this frame.
    pub outcome: Option<TaskOutcome>,
    /// The task, a bench's or the application's, closed by its Cancel:
    /// what it did while open is taken back.
    pub take_back: bool,
    /// A task that holds its edits as one undo step is open after this
    /// frame.
    pub open: bool,
    /// The application's own task ended this frame.
    pub host_end: Option<super::host_tasks::HostTaskEnd>,
    /// What the application's own task asked of the host.
    pub commands: Vec<super::UiCommand>,
}

pub struct TaskPanelInputs<'a> {
    pub active_workbench: ActiveWorkbench,
    pub document: &'a mut core_document::Document,
    pub registry: &'a mut core_document::DocumentService,
    pub host: HostCtxParams,
    pub active_document_object: Option<core_document::FeatureId>,
    pub task: Option<&'a TaskInfo>,
    /// The application's own task, shown while no bench has one open.
    pub host_task: Option<&'a mut super::host_tasks::HostTask>,
    /// The faces selected in the view: each its body and index.
    pub picked_faces: &'a [(core_document::BodyId, u32)],
    pub picked_edges: &'a [(core_document::BodyId, [f32; 3])],
    pub custom_colors: &'a [[f32; 3]],
    pub printing: &'a settings::PrintingSettings,
    pub print_layout: Option<&'a crate::app::print_layout::Layout>,
    pub marking: Option<(usize, usize)>,
}

pub fn draw_task_panel(ui: &mut egui::Ui, inputs: TaskPanelInputs<'_>) -> TaskPanelResult {
    let TaskPanelInputs {
        active_workbench,
        document,
        registry,
        host,
        active_document_object,
        task,
        host_task,
        picked_faces,
        picked_edges,
        custom_colors,
        printing,
        print_layout,
        marking,
    } = inputs;
    let mut result = TaskPanelResult::default();
    let request = keyboard_request(ui);

    // A bench's task first; the application's own while none is open.
    let Some(info) = task else {
        let Some(host_task) = host_task else {
            return result;
        };
        result.open = true;
        let header = Header {
            title: host_task.title(document),
            subtitle: String::new(),
            status: None,
            confirmable: host_task.confirmable(),
        };
        panel(ui, &header, request, |ui, request| {
            result.take_back = request.cancel;
            result.host_end = host_task.show(
                ui,
                super::host_tasks::HostTaskInputs {
                    document,
                    picked_faces,
                    picked_edges,
                    registry: &*registry,
                    custom_colors,
                    printing,
                    print_layout,
                    marking,
                },
                request.accept,
                request.cancel,
                &mut result.commands,
            );
        });
        if result.host_end.is_some() {
            result.open = false;
        }
        result.take_back &= matches!(
            result.host_end,
            Some(super::host_tasks::HostTaskEnd::Cancelled)
        );
        return result;
    };
    let Ok(wb) = registry.workbench_mut(&active_workbench.0) else {
        return result;
    };
    // A stepwise task leaves each edit its own undo step.
    result.open = !info.stepwise;
    let header = Header {
        title: info.title.clone(),
        subtitle: info.subtitle.clone(),
        status: info.status.clone(),
        confirmable: info.confirmable,
    };
    panel(ui, &header, request, |ui, request| {
        let mut ctx = panel_ctx(document, &host, active_document_object);
        match wb.ui_task_panel(ui, &mut ctx, request) {
            TaskOutcome::Open => {}
            outcome => {
                result.take_back = request.cancel && outcome == TaskOutcome::Cancelled;
                result.outcome = Some(outcome);
                result.open = false;
            }
        }
        result.writeback = PanelWriteback::take(&mut ctx, active_document_object);
        flush_ctx_logs(&mut ctx);
    });
    result
}

/// The strip under the view the active bench fills, when it has one: a
/// band across the view's bottom, above the status bar.
pub fn draw_viewport_strip(
    ui: &mut egui::Ui,
    active_workbench: &ActiveWorkbench,
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    host: &HostCtxParams,
    active_document_object: Option<core_document::FeatureId>,
) -> Option<PanelWriteback> {
    let wb = registry.workbench_mut(&active_workbench.0).ok()?;
    if !wb.has_viewport_strip() {
        return None;
    }
    let mut writeback = None;
    egui::Panel::bottom("bench_strip")
        .frame(
            egui::Frame::new()
                .fill(BG1)
                .inner_margin(egui::Margin::symmetric(12, 6)),
        )
        .show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter().hline(
                rect.x_range(),
                rect.top() - 6.0,
                egui::Stroke::new(1.0, BORDER),
            );
            let mut ctx = panel_ctx(document, host, active_document_object);
            wb.ui_viewport_strip(ui, &mut ctx);
            writeback = Some(PanelWriteback::take(&mut ctx, active_document_object));
            flush_ctx_logs(&mut ctx);
        });
    writeback
}

/// Enter, when no text field owns it: a focused field keeps its own Enter
/// and the next press arrives. Escape never ends a task: it stays with the
/// bench, which puts away a tool, a pick or a selection with it.
fn keyboard_request(ui: &egui::Ui) -> TaskRequest {
    let no_focus = ui.ctx().memory(|m| m.focused()).is_none();
    let mut request = TaskRequest::default();
    if no_focus {
        ui.ctx().input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                request.accept = true;
            }
        });
    }
    request
}

/// What the panel's header shows.
struct Header {
    title: String,
    subtitle: String,
    status: Option<core_document::TaskStatus>,
    /// Cancel beside Done; otherwise Done alone.
    confirmable: bool,
}

/// The panel: its header and the task's body, which gets the request the
/// keys and buttons made.
fn panel(
    ui: &mut egui::Ui,
    header: &Header,
    mut request: TaskRequest,
    body: impl FnOnce(&mut egui::Ui, TaskRequest),
) {
    egui::Panel::right("task_panel")
        .resizable(true)
        .default_size(TASK_PANEL_WIDTH)
        .size_range(260.0..=480.0)
        .frame(egui::Frame::new().fill(BG1))
        .show(ui, |ui| {
            // Nothing paints past the panel, whatever a bench's editor asks.
            ui.set_clip_rect(ui.max_rect());
            let rect = ui.max_rect().intersect(ui.clip_rect());
            ui.painter().vline(
                rect.left() + 0.5,
                rect.y_range(),
                egui::Stroke::new(1.0, BORDER),
            );
            let done = egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 14,
                    right: 12,
                    top: 12,
                    bottom: 10,
                })
                .show(ui, |ui| header_row(ui, header))
                .inner;
            request.accept |= done.accept;
            request.cancel |= done.cancel;
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
                            body(ui, request);
                        });
                });
        });
}

/// The header's row: the title, what it edits and the state on the left,
/// Cancel and Done at the right. Which button was pressed.
fn header_row(ui: &mut egui::Ui, header: &Header) -> TaskRequest {
    let mut pressed = TaskRequest::default();
    let row = ui.available_rect_before_wrap();
    // The buttons first, from the right edge, so the text knows its room.
    let mut buttons = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(row)
            .layout(egui::Layout::right_to_left(egui::Align::Min)),
    );
    buttons.spacing_mut().item_spacing.x = SPACE_2;
    if primary_button(&mut buttons, "Done")
        .on_hover_text("Keep the changes and close (Enter)")
        .clicked()
    {
        pressed.accept = true;
    }
    if header.confirmable
        && secondary_button(&mut buttons, "Cancel")
            .on_hover_text("Put everything back as it was and close")
            .clicked()
    {
        pressed.cancel = true;
    }
    let taken = buttons.min_rect();
    let text_rect = egui::Rect::from_min_max(
        row.min,
        egui::pos2((taken.left() - SPACE_2).max(row.left()), row.bottom()),
    );
    let mut text = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    text.spacing_mut().item_spacing.y = 2.0;
    text.add(
        egui::Label::new(
            RichText::new(&header.title)
                .font(sans_semibold(FONT_MD))
                .color(TEXT1),
        )
        .truncate(),
    );
    // A subtitle that only repeats the title ("Pad" named Pad) is left out.
    if !header.subtitle.is_empty() && header.subtitle != header.title {
        text.add(
            egui::Label::new(
                RichText::new(&header.subtitle)
                    .font(sans(FONT_XS))
                    .color(TEXT3),
            )
            .truncate(),
        );
    }
    if let Some(status) = &header.status {
        text.add_space(SPACE_1);
        let note = match status.tone {
            StatusTone::Info => Note::Info,
            StatusTone::Good => Note::Success,
            StatusTone::Warning => Note::Warning,
            StatusTone::Bad => Note::Error,
        };
        status_pill(&mut text, note, &status.text, &status.detail);
    }
    let used = text.min_rect().union(taken);
    ui.allocate_rect(
        egui::Rect::from_min_max(row.min, egui::pos2(row.right(), used.bottom())),
        egui::Sense::hover(),
    );
    pressed
}
