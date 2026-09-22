//! The floating view toolbar at the top of the viewport: fit, standard
//! views, draw style and projection.

use egui::{Align2, Area, Context, Order, Vec2};
use settings::{DrawStyle, ProjectionMode};
use ui_kit::tokens::*;
use ui_kit::widgets::{ToolButtonState, tool_button, vseparator};

use super::UiCommand;
use crate::orientation_cube::CameraSnapView;

const BUTTON: f32 = 28.0;

enum Item {
    Button {
        icon: &'static str,
        label: &'static str,
        on: bool,
        planned: Option<&'static str>,
        command: Option<UiCommand>,
    },
    Sep,
}

fn button(icon: &'static str, label: &'static str, command: UiCommand) -> Item {
    Item::Button {
        icon,
        label,
        on: false,
        planned: None,
        command: Some(command),
    }
}

fn toggled(icon: &'static str, label: &'static str, on: bool, command: UiCommand) -> Item {
    Item::Button {
        icon,
        label,
        on,
        planned: None,
        command: Some(command),
    }
}

fn planned(icon: &'static str, label: &'static str, note: &'static str) -> Item {
    Item::Button {
        icon,
        label,
        on: false,
        planned: Some(note),
        command: None,
    }
}

/// Draws the pill and pushes the commands of any clicked button.
pub fn draw_view_toolbar(
    ctx: &Context,
    viewport: egui::Rect,
    projection: ProjectionMode,
    draw_style: DrawStyle,
    commands: &mut Vec<UiCommand>,
) {
    let ortho = projection == ProjectionMode::Orthographic;
    let items = [
        button("fit-all", "Fit all", UiCommand::FitView),
        button("fit-selection", "Fit selection", UiCommand::FitSelection),
        Item::Sep,
        button(
            "view-iso",
            "Isometric",
            UiCommand::CameraSnap(CameraSnapView::FrontTopRight),
        ),
        button(
            "view-front",
            "Front",
            UiCommand::CameraSnap(CameraSnapView::Front),
        ),
        button(
            "view-top",
            "Top",
            UiCommand::CameraSnap(CameraSnapView::Top),
        ),
        button(
            "view-right",
            "Right",
            UiCommand::CameraSnap(CameraSnapView::Right),
        ),
        button(
            "view-rear",
            "Rear",
            UiCommand::CameraSnap(CameraSnapView::Rear),
        ),
        button(
            "view-bottom",
            "Bottom",
            UiCommand::CameraSnap(CameraSnapView::Bottom),
        ),
        button(
            "view-left",
            "Left",
            UiCommand::CameraSnap(CameraSnapView::Left),
        ),
        Item::Sep,
        toggled(
            "draw-style-shaded",
            "Shaded with edges",
            draw_style == DrawStyle::ShadedEdges,
            UiCommand::SetDrawStyle(DrawStyle::ShadedEdges),
        ),
        toggled(
            "draw-style-flat",
            "Shaded",
            draw_style == DrawStyle::Shaded,
            UiCommand::SetDrawStyle(DrawStyle::Shaded),
        ),
        toggled(
            "draw-style-wireframe",
            "Wireframe",
            draw_style == DrawStyle::Wireframe,
            UiCommand::SetDrawStyle(DrawStyle::Wireframe),
        ),
        // PLANNED: a clipping plane through the scene.
        planned(
            "clipping-plane",
            "Clipping plane",
            "cuts the view with a plane",
        ),
        Item::Sep,
        toggled(
            "view-orthographic",
            "Orthographic",
            ortho,
            UiCommand::SetProjection(ProjectionMode::Orthographic),
        ),
        toggled(
            "view-perspective",
            "Perspective",
            !ortho,
            UiCommand::SetProjection(ProjectionMode::Perspective),
        ),
    ];

    Area::new(egui::Id::new("view_toolbar"))
        .order(Order::Foreground)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(viewport.center_top() + Vec2::new(0.0, 10.0))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(OVERLAY_CARD)
                .stroke(egui::Stroke::new(1.0, BORDER))
                .corner_radius(7)
                .inner_margin(3)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for item in items {
                            match item {
                                Item::Sep => {
                                    ui.add_space(3.0);
                                    vseparator(ui, 18.0);
                                    ui.add_space(3.0);
                                }
                                Item::Button {
                                    icon,
                                    label,
                                    on,
                                    planned,
                                    command,
                                } => {
                                    let state = ToolButtonState {
                                        enabled: command.is_some(),
                                        active: on,
                                        planned,
                                        menu: false,
                                    };
                                    if tool_button(ui, icon, label, BUTTON, state).clicked()
                                        && let Some(command) = command
                                    {
                                        commands.push(command);
                                    }
                                }
                            }
                        }
                    });
                });
        });
}
